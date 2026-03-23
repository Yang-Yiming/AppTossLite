use std::fs;
use std::path::PathBuf;

use crate::core::config::Config;
use crate::core::error::{Result, TossError};
use crate::core::sign::{self, ProvisioningProfileInspection};

#[derive(Debug, Clone)]
pub struct ProfileDirInfo {
    pub path: PathBuf,
    pub file_count: usize,
}

#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub config_path: PathBuf,
    pub temp_bundle_prefix: Option<String>,
    pub team_id: Option<String>,
    pub default_device: Option<String>,
    pub default_project: Option<String>,
    pub device_aliases: Vec<(String, String)>,
    pub projects: Vec<(String, crate::core::config::ProjectConfig)>,
    pub profile_dirs: Vec<ProfileDirInfo>,
    pub profile_inspections: std::result::Result<Vec<ProvisioningProfileInspection>, String>,
}

pub fn collect(config: &Config) -> Result<StateSnapshot> {
    let config_path = Config::path()?;
    let profile_dirs = provisioning_profile_dirs()?
        .into_iter()
        .map(|path| {
            let file_count = fs::read_dir(&path)
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
            ProfileDirInfo { path, file_count }
        })
        .collect();

    Ok(StateSnapshot {
        config_path,
        temp_bundle_prefix: config.signing.temp_bundle_prefix.clone(),
        team_id: config.signing.team_id.clone(),
        default_device: config.defaults.device.clone(),
        default_project: config.defaults.project.clone(),
        device_aliases: config
            .devices
            .aliases
            .iter()
            .map(|(alias, udid)| (alias.clone(), udid.clone()))
            .collect(),
        projects: config
            .projects
            .iter()
            .map(|(name, project)| (name.clone(), project.clone()))
            .collect(),
        profile_dirs,
        profile_inspections: sign::inspect_provisioning_profiles().map_err(|e| e.to_string()),
    })
}

pub fn provisioning_profile_dirs() -> Result<Vec<PathBuf>> {
    let home = dirs::home_dir()
        .ok_or_else(|| TossError::Config("cannot determine home directory".into()))?;

    Ok([
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ]
    .into_iter()
    .filter(|path| path.is_dir())
    .collect())
}
