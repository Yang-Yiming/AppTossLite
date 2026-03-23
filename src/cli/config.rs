use crate::core::config::Config;
use crate::core::error::{Result, TossError};

pub fn show(config: &Config) -> Result<()> {
    let content = toml::to_string_pretty(config)?;
    println!("{}", content);
    Ok(())
}

pub fn path() -> Result<()> {
    let path = Config::path()?;
    println!("{}", path.display());
    Ok(())
}

pub fn set_default_device(config: &mut Config, name: &str) -> Result<()> {
    // Verify the alias or UDID is valid
    let is_alias = config.devices.aliases.contains_key(name);
    let is_udid = config.devices.aliases.values().any(|v| v == name);

    if !is_alias && !is_udid {
        return Err(TossError::Config(format!(
            "unknown device '{}' — use `toss devices alias` to create an alias first, or provide a UDID",
            name
        )));
    }

    config.defaults.device = Some(name.to_string());
    config.save()?;
    println!("Default device set to '{}'", name);
    Ok(())
}

pub fn set_default_project(config: &mut Config, name: &str) -> Result<()> {
    if !config.projects.contains_key(name) {
        return Err(TossError::Config(format!(
            "unknown project '{}' — register it with `toss projects add` first",
            name
        )));
    }

    config.defaults.project = Some(name.to_string());
    config.save()?;
    println!("Default project set to '{}'", name);
    Ok(())
}

pub fn set_temp_bundle_prefix(config: &mut Config, prefix: &str) -> Result<()> {
    let trimmed = prefix.trim().trim_matches('.');
    if trimmed.is_empty() {
        return Err(TossError::Config(
            "temp bundle prefix cannot be empty".into(),
        ));
    }

    let is_valid = trimmed.split('.').all(|part| {
        !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });

    if !is_valid {
        return Err(TossError::Config(format!(
            "invalid temp bundle prefix '{}' — use dot-separated ASCII letters/numbers/hyphens",
            prefix
        )));
    }

    config.signing.temp_bundle_prefix = Some(trimmed.to_string());
    config.save()?;
    println!("Temp bundle prefix set to '{}'", trimmed);
    Ok(())
}

pub fn set_team_id(config: &mut Config, team_id: &str) -> Result<()> {
    let trimmed = team_id.trim().trim_matches('.');
    if trimmed.is_empty() {
        return Err(TossError::Config("team id cannot be empty".into()));
    }

    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(TossError::Config(format!(
            "invalid team id '{}' — use ASCII letters and numbers only",
            team_id
        )));
    }

    config.signing.team_id = Some(trimmed.to_string());
    config.save()?;
    println!("Team ID set to '{}'", trimmed);
    Ok(())
}
