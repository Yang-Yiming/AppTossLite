use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::config::Config;
use super::error::{Result, TossError};

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
        if entry.path().extension().is_some_and(|ext| ext == "xcworkspace") {
            return Ok((entry.path(), true));
        }
    }
    // Fallback to .xcodeproj
    for entry in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        if entry.path().extension().is_some_and(|ext| ext == "xcodeproj") {
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
    let flag = if is_workspace { "-workspace" } else { "-project" };
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
pub fn select_scheme(schemes: Vec<String>) -> Result<String> {
    match schemes.len() {
        0 => Err(TossError::Project("no schemes available".into())),
        1 => Ok(schemes[0].clone()),
        _ => {
            use dialoguer::Select;
            let selection = Select::new()
                .with_prompt("Select scheme")
                .items(&schemes)
                .default(0)
                .interact()
                .map_err(|e| TossError::UserCancelled(e.to_string()))?;
            Ok(schemes[selection].clone())
        }
    }
}
