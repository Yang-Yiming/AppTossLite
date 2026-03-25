use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::config::{Config, ProjectConfig};
use super::error::{Result, TossError};
use super::interaction::{WorkflowAdapter, choose_index};

#[derive(Debug, Clone)]
pub struct AddedProject {
    pub name: String,
    pub build_dir: PathBuf,
    pub source_dir: Option<PathBuf>,
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
        path: source_dir.as_ref().map(|p| p.to_string_lossy().to_string()),
        build_dir: build_dir.to_string_lossy().to_string(),
        bundle_id: bundle_id.clone(),
        app_name: app_name.clone(),
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
        build_dir,
        source_dir,
        app_name,
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
