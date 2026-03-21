use std::path::Path;

use crate::core::actions;
use crate::core::config::Config;
use crate::core::error::Result;

pub fn install(
    config: &Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: bool,
    verbose: bool,
) -> Result<()> {
    actions::install(config, project, device, Some(prebuilt), verbose)
}

pub fn launch(config: &Config, project: Option<&str>, device: Option<&str>) -> Result<()> {
    actions::launch(config, project, device)
}

pub fn run(
    config: &Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: bool,
    verbose: bool,
) -> Result<()> {
    actions::run(config, project, device, Some(prebuilt), verbose)
}

pub fn sign(
    config: &Config,
    ipa: &str,
    device: Option<&str>,
    identity: Option<&str>,
    profile: Option<&str>,
    launch: bool,
) -> Result<()> {
    actions::sign_ipa(config, Path::new(ipa), device, identity, profile, launch)
}
