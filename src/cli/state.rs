use std::fs;
use std::path::PathBuf;

use crate::core::config::Config;
use crate::core::error::Result;
use crate::core::sign;

pub fn show(config: &Config) -> Result<()> {
    let config_path = Config::path()?;
    println!("Local state");
    println!("  config file: {}", config_path.display());
    println!("  stored here: defaults, device aliases, projects, temp_bundle_prefix, team_id");
    println!(
        "  temp_bundle_prefix: {}",
        config
            .signing
            .temp_bundle_prefix
            .as_deref()
            .unwrap_or("<unset>")
    );
    println!(
        "  team_id: {}",
        config.signing.team_id.as_deref().unwrap_or("<unset>")
    );
    println!(
        "  default_device: {}",
        config.defaults.device.as_deref().unwrap_or("<unset>")
    );
    println!(
        "  default_project: {}",
        config.defaults.project.as_deref().unwrap_or("<unset>")
    );
    println!();

    println!("Device aliases ({})", config.devices.aliases.len());
    if config.devices.aliases.is_empty() {
        println!("  <none>");
    } else {
        for (alias, udid) in &config.devices.aliases {
            println!("  {} -> {}", alias, udid);
        }
    }
    println!();

    println!("Projects ({})", config.projects.len());
    if config.projects.is_empty() {
        println!("  <none>");
    } else {
        for (name, project) in &config.projects {
            println!("  {}", name);
            println!("    build_dir: {}", project.build_dir);
            if let Some(path) = &project.path {
                println!("    source: {}", path);
            }
            if let Some(bundle_id) = &project.bundle_id {
                println!("    bundle_id: {}", bundle_id);
            }
            if let Some(app_name) = &project.app_name {
                println!("    app_name: {}", app_name);
            }
        }
    }
    println!();

    let profile_dirs = provisioning_profile_dirs()?;
    println!("Provisioning profile dirs ({})", profile_dirs.len());
    if profile_dirs.is_empty() {
        println!("  <none>");
    } else {
        for dir in &profile_dirs {
            let file_count = fs::read_dir(dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "mobileprovision")
                })
                .count();
            println!("  {} ({} files)", dir.display(), file_count);
        }
        println!("  stored here: downloaded Xcode provisioning profiles");
    }
    println!();

    println!("Provisioning profiles");
    match sign::inspect_provisioning_profiles() {
        Ok(inspections) => {
            if inspections.is_empty() {
                println!("  <none>");
            } else {
                let prefix = config.signing.temp_bundle_prefix.as_deref();
                for inspection in inspections {
                    match inspection.profile {
                        Some(profile) => {
                            let is_temp = prefix
                                .map(|value| profile.bundle_id_pattern.starts_with(value))
                                .unwrap_or(false);
                            let marker = if is_temp { " [temp]" } else { "" };
                            println!("  {}{}", profile.name, marker);
                            println!("    bundle: {}", profile.bundle_id_pattern);
                            if !profile.team_ids.is_empty() {
                                println!("    team: {}", profile.team_ids.join(", "));
                            }
                            println!("    path: {}", profile.path.display());
                        }
                        None => {
                            println!("  <parse failed>");
                            println!("    path: {}", inspection.path.display());
                            println!(
                                "    error: {}",
                                inspection.error.as_deref().unwrap_or("unknown error")
                            );
                        }
                    }
                }
            }
        }
        Err(err) => println!("  unavailable: {}", err),
    }

    Ok(())
}

fn provisioning_profile_dirs() -> Result<Vec<PathBuf>> {
    let home = dirs::home_dir().ok_or_else(|| {
        crate::core::error::TossError::Config("cannot determine home directory".into())
    })?;

    Ok([
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ]
    .into_iter()
    .filter(|path| path.is_dir())
    .collect())
}
