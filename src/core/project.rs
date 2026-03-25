use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::config::{Config, ProjectConfig, ProjectKind};
use super::error::{Result, TossError};
use super::interaction::{WorkflowAdapter, choose_index};
use super::sign;

#[derive(Debug, Clone)]
pub struct AddedProject {
    pub name: String,
    pub kind: ProjectKind,
    pub build_dir: Option<PathBuf>,
    pub source_dir: Option<PathBuf>,
    pub cached_ipa_path: Option<PathBuf>,
    pub original_name: Option<String>,
    pub app_name: Option<String>,
    pub bundle_id: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct RemovedProject {
    pub name: String,
    pub cleared_default: bool,
}

/// Extract bundle ID from an .app's Info.plist using plutil.
pub fn extract_bundle_id(app_path: &Path) -> Result<String> {
    let plist = app_path.join("Info.plist");
    if !plist.exists() {
        return Err(TossError::Project(format!(
            "Info.plist not found at {}",
            plist.display()
        )));
    }

    let output = Command::new("plutil")
        .args([
            "-extract",
            "CFBundleIdentifier",
            "raw",
            &plist.to_string_lossy(),
        ])
        .output()?;

    if !output.status.success() {
        return Err(TossError::Project(format!(
            "plutil failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resolve a project alias to its (app_path, bundle_id).
pub fn resolve_project(config: &Config, project: &str) -> Result<(PathBuf, String)> {
    let proj = config.projects.get(project).ok_or_else(|| {
        TossError::Project(format!(
            "unknown project '{}' — register it with `toss projects add`",
            project
        ))
    })?;

    if proj.is_ipa() {
        return Err(TossError::Project(format!(
            "project '{}' is an IPA project and cannot be resolved as a prebuilt app bundle",
            project
        )));
    }

    let build_dir = PathBuf::from(&proj.build_dir);

    let app_name = match &proj.app_name {
        Some(name) => name.clone(),
        None => find_app_in_dir(&build_dir)?,
    };

    let app_path = build_dir.join(&app_name);

    let bundle_id = match &proj.bundle_id {
        Some(bid) => bid.clone(),
        None => extract_bundle_id(&app_path)?,
    };

    Ok((app_path, bundle_id))
}

pub fn add_project(
    config: &mut Config,
    path: &str,
    alias: Option<&str>,
    adapter: &mut impl WorkflowAdapter,
) -> Result<AddedProject> {
    if let Some(alias) = alias
        && config.projects.contains_key(alias)
    {
        return Err(TossError::Project(format!(
            "project '{}' already exists",
            alias
        )));
    }

    let input_path = PathBuf::from(shellexpand(path));

    if !input_path.exists() {
        return Err(TossError::Project(format!("'{}' does not exist", path)));
    }

    let (source_dir, build_dir, project_name) =
        if input_path.extension().is_some_and(|ext| ext == "app") {
            let parent = input_path.parent().unwrap().to_path_buf();
            (None, parent, None)
        } else if let Some(proj_name) = find_xcodeproj(&input_path) {
            let build = resolve_build_from_source(&input_path, adapter)?;
            (Some(input_path.clone()), build, Some(proj_name))
        } else if input_path.is_dir() {
            (None, input_path.clone(), None)
        } else {
            return Err(TossError::Project(format!(
                "'{}' is not a directory or .app bundle",
                path
            )));
        };

    let name = match alias {
        Some(a) => a.to_string(),
        None => {
            let base = project_name.unwrap_or_else(|| {
                build_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "project".to_string())
            });
            base.to_lowercase()
                .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "")
        }
    };

    let app_name = find_app_in_dir(&build_dir).ok();
    let bundle_id = app_name.as_ref().and_then(|app| {
        let app_path = build_dir.join(app);
        extract_bundle_id(&app_path).ok()
    });

    let project = ProjectConfig {
        kind: ProjectKind::Xcode,
        path: source_dir.as_ref().map(|p| p.to_string_lossy().to_string()),
        build_dir: build_dir.to_string_lossy().to_string(),
        bundle_id: bundle_id.clone(),
        app_name: app_name.clone(),
        ipa_path: None,
        original_name: None,
        last_tossed_at: None,
    };

    if config.projects.contains_key(&name) {
        return Err(TossError::Project(format!(
            "project '{}' already exists",
            name
        )));
    }

    let is_first = config.projects.is_empty();
    config.projects.insert(name.clone(), project);
    if is_first {
        config.defaults.project = Some(name.clone());
    }

    config.save()?;

    Ok(AddedProject {
        name,
        kind: ProjectKind::Xcode,
        build_dir: Some(build_dir),
        source_dir,
        cached_ipa_path: None,
        original_name: None,
        app_name,
        bundle_id,
        is_default: is_first,
    })
}

pub fn add_ipa_project(
    config: &mut Config,
    path: &str,
    alias: Option<&str>,
) -> Result<AddedProject> {
    let input_path = PathBuf::from(shellexpand(path));

    if !input_path.exists() {
        return Err(TossError::Project(format!("'{}' does not exist", path)));
    }

    if !input_path.is_file() || input_path.extension().is_none_or(|ext| ext != "ipa") {
        return Err(TossError::Project(format!(
            "'{}' is not an .ipa file",
            path
        )));
    }

    let original_name = input_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| TossError::Project(format!("cannot determine file name for '{}'", path)))?;

    let name = alias
        .map(str::to_string)
        .unwrap_or_else(|| sanitize_alias_from_name(&original_name));

    if config.projects.contains_key(&name) {
        return Err(TossError::Project(format!(
            "project '{}' already exists",
            name
        )));
    }

    let cached_path = cache_ipa_file(&input_path, &name)?;
    let bundle_id = inspect_ipa_bundle_id(&input_path).ok();

    let project = ProjectConfig {
        kind: ProjectKind::Ipa,
        path: None,
        build_dir: String::new(),
        bundle_id: bundle_id.clone(),
        app_name: None,
        ipa_path: Some(cached_path.to_string_lossy().to_string()),
        original_name: Some(original_name.clone()),
        last_tossed_at: None,
    };

    let is_first = config.projects.is_empty();
    config.projects.insert(name.clone(), project);
    if is_first {
        config.defaults.project = Some(name.clone());
    }

    config.save()?;

    Ok(AddedProject {
        name,
        kind: ProjectKind::Ipa,
        build_dir: None,
        source_dir: None,
        cached_ipa_path: Some(cached_path),
        original_name: Some(original_name),
        app_name: None,
        bundle_id,
        is_default: is_first,
    })
}

pub fn remove_project(config: &mut Config, alias: &str) -> Result<RemovedProject> {
    if config.projects.remove(alias).is_none() {
        return Err(TossError::Project(format!(
            "no project named '{}' found",
            alias
        )));
    }

    let cleared_default = config.defaults.project.as_deref() == Some(alias);
    if cleared_default {
        config.defaults.project = None;
    }

    config.save()?;
    Ok(RemovedProject {
        name: alias.to_string(),
        cleared_default,
    })
}

/// Find the DerivedData build directory for an Xcode project.
///
/// Given a source directory containing a `.xcodeproj`, scans
/// `~/Library/Developer/Xcode/DerivedData/` for a matching folder
/// and returns the `Build/Products/Debug-iphoneos/` path.
pub fn find_derived_data_build(source_dir: &Path) -> Result<Vec<PathBuf>> {
    // Find the .xcodeproj to derive the project name
    let xcodeproj = fs::read_dir(source_dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "xcodeproj"))
        .ok_or_else(|| {
            TossError::Project(format!("no .xcodeproj found in {}", source_dir.display()))
        })?;

    let project_name = xcodeproj
        .path()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();

    let derived_data = dirs::home_dir()
        .ok_or_else(|| TossError::Project("cannot determine home directory".into()))?
        .join("Library/Developer/Xcode/DerivedData");

    if !derived_data.is_dir() {
        return Err(TossError::Project(format!(
            "DerivedData not found at {}",
            derived_data.display()
        )));
    }

    // DerivedData folders are named like "ProjectName-abcdef1234"
    let prefix = format!("{}-", project_name);
    let mut matches = Vec::new();

    for entry in fs::read_dir(&derived_data)?.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) {
            let build_products = entry.path().join("Build/Products/Debug-iphoneos");
            if build_products.is_dir() {
                // Verify there's at least one .app inside
                let has_app = fs::read_dir(&build_products)
                    .ok()
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .any(|e| e.path().extension().is_some_and(|ext| ext == "app"))
                    })
                    .unwrap_or(false);
                if has_app {
                    matches.push(build_products);
                }
            }
        }
    }

    if matches.is_empty() {
        return Err(TossError::Project(format!(
            "no DerivedData build found for '{}' — build the project in Xcode first",
            project_name
        )));
    }

    Ok(matches)
}

/// Find a single .app bundle in a build directory.
pub fn find_app_in_dir(build_dir: &Path) -> Result<String> {
    let entries: Vec<_> = fs::read_dir(build_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "app"))
        .collect();

    match entries.len() {
        0 => Err(TossError::Project(format!(
            "no .app found in {}",
            build_dir.display()
        ))),
        1 => Ok(entries[0].file_name().to_string_lossy().to_string()),
        _ => Err(TossError::Project(format!(
            "multiple .app bundles found in {} — specify app_name in config",
            build_dir.display()
        ))),
    }
}

/// Find .xcworkspace or .xcodeproj in a directory.
pub fn find_xcode_project(dir: &Path) -> Result<(PathBuf, bool)> {
    // Prefer .xcworkspace
    for entry in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext == "xcworkspace")
        {
            return Ok((entry.path(), true));
        }
    }
    // Fallback to .xcodeproj
    for entry in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext == "xcodeproj")
        {
            return Ok((entry.path(), false));
        }
    }
    Err(TossError::Project(format!(
        "no .xcworkspace or .xcodeproj found in {}",
        dir.display()
    )))
}

/// List schemes from an Xcode project.
pub fn list_schemes(project_path: &Path, is_workspace: bool) -> Result<Vec<String>> {
    let flag = if is_workspace {
        "-workspace"
    } else {
        "-project"
    };
    let output = Command::new("xcodebuild")
        .args(["-list", flag, &project_path.to_string_lossy()])
        .output()?;

    if !output.status.success() {
        return Err(TossError::Project(format!(
            "xcodebuild -list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut schemes = Vec::new();
    let mut in_schemes = false;

    for line in stdout.lines() {
        if line.trim() == "Schemes:" {
            in_schemes = true;
            continue;
        }
        if in_schemes {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            schemes.push(trimmed.to_string());
        }
    }

    if schemes.is_empty() {
        return Err(TossError::Project("no schemes found".into()));
    }

    Ok(schemes)
}

/// Select a scheme interactively or auto-select if only one.
pub fn select_scheme(schemes: Vec<String>, adapter: &mut impl WorkflowAdapter) -> Result<String> {
    match schemes.len() {
        0 => Err(TossError::Project("no schemes available".into())),
        1 => Ok(schemes[0].clone()),
        _ => {
            let selection = choose_index(
                adapter,
                "Select scheme",
                &schemes,
                TossError::Project(
                    "multiple schemes found — specify one via an interactive adapter".into(),
                ),
            )?;
            Ok(schemes[selection].clone())
        }
    }
}

fn find_xcodeproj(dir: &PathBuf) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find_map(|e| {
            let path = e.path();
            if path.extension().is_some_and(|ext| ext == "xcodeproj") {
                path.file_stem().map(|s| s.to_string_lossy().to_string())
            } else {
                None
            }
        })
}

pub fn managed_ipa_path(config: &Config, project: &str) -> Result<PathBuf> {
    let proj = config.projects.get(project).ok_or_else(|| {
        TossError::Project(format!(
            "unknown project '{}' — register it with `toss projects add`",
            project
        ))
    })?;

    if !proj.is_ipa() {
        return Err(TossError::Project(format!(
            "project '{}' is not an IPA project",
            project
        )));
    }

    let ipa_path = proj.ipa_path.as_deref().ok_or_else(|| {
        TossError::Project(format!(
            "project '{}' is missing its cached ipa path",
            project
        ))
    })?;

    Ok(PathBuf::from(ipa_path))
}

pub fn toss_cache_dir() -> Result<PathBuf> {
    let base = dirs::cache_dir()
        .ok_or_else(|| TossError::Config("cannot determine cache directory".into()))?;
    Ok(base.join("toss").join("ipas"))
}

fn cache_ipa_file(input_path: &Path, alias: &str) -> Result<PathBuf> {
    let cache_dir = toss_cache_dir()?;
    fs::create_dir_all(&cache_dir)?;
    let file_name = format!("{}-{}.ipa", alias, short_hash(input_path));
    let destination = cache_dir.join(file_name);
    fs::copy(input_path, &destination)?;
    Ok(destination)
}

fn inspect_ipa_bundle_id(path: &Path) -> Result<String> {
    let extracted = sign::unzip_ipa(path)?;
    extract_bundle_id(&extracted.app_path)
}

fn sanitize_alias_from_name(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string())
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "")
}

fn short_hash(path: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::UNIX_EPOCH;

    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    if let Ok(metadata) = fs::metadata(path) {
        metadata.len().hash(&mut hasher);
        if let Ok(modified) = metadata.modified()
            && let Ok(duration) = modified.duration_since(UNIX_EPOCH)
        {
            duration.as_secs().hash(&mut hasher);
        }
    }
    format!("{:08x}", hasher.finish() as u32)
}

fn resolve_build_from_source(
    source_dir: &Path,
    adapter: &mut impl WorkflowAdapter,
) -> Result<PathBuf> {
    let matches = find_derived_data_build(source_dir)?;

    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            let items: Vec<String> = matches.iter().map(|p| p.display().to_string()).collect();
            let selection = choose_index(
                adapter,
                "Multiple build directories found — choose one",
                &items,
                TossError::Project(
                    "multiple build directories found — use the TUI to choose one".into(),
                ),
            )?;
            Ok(matches.into_iter().nth(selection).unwrap())
        }
    }
}

fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().to_string();
    }
    path.to_string()
}
