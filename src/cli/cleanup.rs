use std::fs;

use crate::core::config::Config;
use crate::core::error::Result;
use crate::core::sign;

pub fn run(config: &Config) -> Result<()> {
    let Some(prefix) = config.signing.temp_bundle_prefix.as_deref() else {
        println!("No temp bundle prefix configured. Nothing to clean automatically.");
        if let Ok(path) = Config::path() {
            println!("Config path: {}", path.display());
        }
        return Ok(());
    };

    let inspections = sign::inspect_provisioning_profiles()?;
    let to_remove: Vec<_> = inspections
        .into_iter()
        .filter_map(|inspection| inspection.profile)
        .filter(|profile| profile.bundle_id_pattern.starts_with(prefix))
        .collect();

    if to_remove.is_empty() {
        println!(
            "No temporary provisioning profiles found for prefix '{}'.",
            prefix
        );
        return Ok(());
    }

    println!(
        "Removing {} temporary provisioning profile(s) for prefix '{}':",
        to_remove.len(),
        prefix
    );

    for profile in &to_remove {
        println!("  {} ({})", profile.name, profile.path.display());
    }

    for profile in &to_remove {
        if profile.path.exists() {
            fs::remove_file(&profile.path)?;
        }
    }

    println!("Cleanup complete.");
    Ok(())
}
