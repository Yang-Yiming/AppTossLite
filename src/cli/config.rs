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
