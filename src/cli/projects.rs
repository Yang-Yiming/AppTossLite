use std::path::PathBuf;

use crate::core::config::{Config, ProjectConfig};
use crate::core::error::{Result, TossError};
use crate::core::project::{extract_bundle_id, find_app_in_dir};

pub fn add(config: &mut Config, path: &str, alias: Option<&str>) -> Result<()> {
    let build_dir = PathBuf::from(path);
    if !build_dir.is_dir() {
        return Err(TossError::Project(format!("'{}' is not a directory", path)));
    }

    // Derive alias from directory name if not provided
    let name = match alias {
        Some(a) => a.to_string(),
        None => build_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".to_string())
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', ""),
    };

    // Auto-detect app_name
    let app_name = find_app_in_dir(&build_dir).ok();

    // Auto-detect bundle ID
    let bundle_id = app_name.as_ref().and_then(|app| {
        let app_path = build_dir.join(app);
        extract_bundle_id(&app_path).ok()
    });

    let project = ProjectConfig {
        path: None,
        build_dir: build_dir.to_string_lossy().to_string(),
        bundle_id: bundle_id.clone(),
        app_name: app_name.clone(),
    };

    config.projects.insert(name.clone(), project);
    config.save()?;

    println!("Added project '{}'", name);
    println!("  build_dir: {}", path);
    if let Some(app) = &app_name {
        println!("  app_name:  {}", app);
    }
    if let Some(bid) = &bundle_id {
        println!("  bundle_id: {}", bid);
    }

    Ok(())
}

pub fn list(config: &Config) -> Result<()> {
    if config.projects.is_empty() {
        println!("No projects registered. Use `toss projects add <build_dir>` to add one.");
        return Ok(());
    }

    for (name, proj) in &config.projects {
        println!("{}:", name);
        println!("  build_dir: {}", proj.build_dir);
        if let Some(app) = &proj.app_name {
            println!("  app_name:  {}", app);
        }
        if let Some(bid) = &proj.bundle_id {
            println!("  bundle_id: {}", bid);
        }
        println!();
    }

    Ok(())
}

pub fn remove(config: &mut Config, alias: &str) -> Result<()> {
    if config.projects.remove(alias).is_none() {
        return Err(TossError::Project(format!(
            "no project named '{}' found",
            alias
        )));
    }

    config.save()?;
    println!("Removed project '{}'", alias);
    Ok(())
}
