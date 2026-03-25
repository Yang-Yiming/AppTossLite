use crate::core::config::Config;
use crate::core::error::Result;
use crate::core::state;
use crate::core::time::format_last_tossed;

pub fn show(config: &Config) -> Result<()> {
    let snapshot = state::collect(config)?;
    println!("Local state");
    println!("  config file: {}", snapshot.config_path.display());
    println!("  stored here: defaults, device aliases, projects, temp_bundle_prefix, team_id");
    println!(
        "  temp_bundle_prefix: {}",
        snapshot.temp_bundle_prefix.as_deref().unwrap_or("<unset>")
    );
    println!(
        "  team_id: {}",
        snapshot.team_id.as_deref().unwrap_or("<unset>")
    );
    println!(
        "  default_device: {}",
        snapshot.default_device.as_deref().unwrap_or("<unset>")
    );
    println!(
        "  default_project: {}",
        snapshot.default_project.as_deref().unwrap_or("<unset>")
    );
    println!();

    println!("Device aliases ({})", snapshot.device_aliases.len());
    if snapshot.device_aliases.is_empty() {
        println!("  <none>");
    } else {
        for (alias, udid) in &snapshot.device_aliases {
            println!("  {} -> {}", alias, udid);
        }
    }
    println!();

    println!("Projects ({})", snapshot.projects.len());
    if snapshot.projects.is_empty() {
        println!("  <none>");
    } else {
        for (name, project) in &snapshot.projects {
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
            println!(
                "    last_tossed_at: {}",
                format_last_tossed(project.last_tossed_at.as_deref())
            );
        }
    }
    println!();

    println!(
        "Provisioning profile dirs ({})",
        snapshot.profile_dirs.len()
    );
    if snapshot.profile_dirs.is_empty() {
        println!("  <none>");
    } else {
        for dir in &snapshot.profile_dirs {
            println!("  {} ({} files)", dir.path.display(), dir.file_count);
        }
        println!("  stored here: downloaded Xcode provisioning profiles");
    }
    println!();

    println!("Provisioning profiles");
    match &snapshot.profile_inspections {
        Ok(inspections) => {
            if inspections.is_empty() {
                println!("  <none>");
            } else {
                let prefix = snapshot.temp_bundle_prefix.as_deref();
                for inspection in inspections {
                    match &inspection.profile {
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
