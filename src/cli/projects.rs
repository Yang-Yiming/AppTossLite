use std::path::PathBuf;

use console::Style;
use dialoguer::Select;

use crate::core::config::{Config, ProjectConfig};
use crate::core::error::{Result, TossError};
use crate::core::project::{extract_bundle_id, find_app_in_dir, find_derived_data_build};

/// Check if a directory contains a .xcodeproj (i.e., it's a source directory).
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

pub fn add(config: &mut Config, path: &str, alias: Option<&str>) -> Result<()> {
    let input_path = PathBuf::from(shellexpand(path));

    if !input_path.exists() {
        return Err(TossError::Project(format!("'{}' does not exist", path)));
    }

    // Determine if this is a source dir (has .xcodeproj), build dir, or .app path
    let (source_dir, build_dir, project_name) =
        if input_path.extension().is_some_and(|ext| ext == "app") {
            // Direct .app path — build_dir is the parent
            let parent = input_path.parent().unwrap().to_path_buf();
            (None, parent, None)
        } else if let Some(proj_name) = find_xcodeproj(&input_path) {
            // Source directory with .xcodeproj — scan DerivedData
            let build = resolve_build_from_source(&input_path)?;
            (Some(input_path.clone()), build, Some(proj_name))
        } else if input_path.is_dir() {
            // Assume it's a build directory directly
            (None, input_path.clone(), None)
        } else {
            return Err(TossError::Project(format!(
                "'{}' is not a directory or .app bundle",
                path
            )));
        };

    // Derive alias
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

    // Auto-detect app_name
    let app_name = find_app_in_dir(&build_dir).ok();

    // Auto-detect bundle ID
    let bundle_id = app_name.as_ref().and_then(|app| {
        let app_path = build_dir.join(app);
        extract_bundle_id(&app_path).ok()
    });

    let project = ProjectConfig {
        path: source_dir.as_ref().map(|p| p.to_string_lossy().to_string()),
        build_dir: build_dir.to_string_lossy().to_string(),
        bundle_id: bundle_id.clone(),
        app_name: app_name.clone(),
    };

    let is_first = config.projects.is_empty();
    config.projects.insert(name.clone(), project);

    // Auto-set default if this is the first project
    if is_first {
        config.defaults.project = Some(name.clone());
    }

    config.save()?;

    let green = Style::new().green().bold();
    println!("{} Added project '{}'", green.apply_to("✓"), name);
    println!("  build_dir: {}", build_dir.display());
    if let Some(src) = &source_dir {
        println!("  source:    {}", src.display());
    }
    if let Some(app) = &app_name {
        println!("  app_name:  {}", app);
    }
    if let Some(bid) = &bundle_id {
        println!("  bundle_id: {}", bid);
    }
    if is_first {
        let dim = Style::new().dim();
        println!("{}", dim.apply_to("  (set as default project)"));
    }

    Ok(())
}

/// Given a source directory containing .xcodeproj, find the build directory via DerivedData.
fn resolve_build_from_source(source_dir: &std::path::Path) -> Result<PathBuf> {
    let matches = find_derived_data_build(source_dir)?;

    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            // Multiple matches — prompt user to choose
            let items: Vec<String> = matches.iter().map(|p| p.display().to_string()).collect();
            let selection = Select::new()
                .with_prompt("Multiple build directories found — choose one")
                .items(&items)
                .default(0)
                .interact()
                .map_err(|e| TossError::UserCancelled(e.to_string()))?;
            Ok(matches.into_iter().nth(selection).unwrap())
        }
    }
}

/// Expand ~ in paths.
fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().to_string();
    }
    path.to_string()
}

pub fn list(config: &Config) -> Result<()> {
    if config.projects.is_empty() {
        println!("No projects registered. Use `toss projects add <path>` to add one.");
        return Ok(());
    }

    let default_project = config.defaults.project.as_deref();

    for (name, proj) in &config.projects {
        let marker = if Some(name.as_str()) == default_project {
            " (default)"
        } else {
            ""
        };
        println!("{}{}:", name, marker);
        println!("  build_dir: {}", proj.build_dir);
        if let Some(src) = &proj.path {
            println!("  source:    {}", src);
        }
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

    // Clear default if it was pointing at the removed project
    if config.defaults.project.as_deref() == Some(alias) {
        config.defaults.project = None;
    }

    config.save()?;
    println!("Removed project '{}'", alias);
    Ok(())
}
