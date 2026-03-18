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
