use std::path::Path;

use console::Style;

use crate::cli::adapters::StrictCliAdapter;
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
    let mut adapter = StrictCliAdapter;
    let result = actions::install(
        config,
        project,
        device,
        Some(prebuilt),
        verbose,
        &mut adapter,
    )?;
    let green = Style::new().green().bold();
    println!(
        "{} Installed '{}' on '{}'.",
        green.apply_to("✓"),
        result.project_name,
        result.device_name
    );
    Ok(())
}

pub fn launch(config: &Config, project: Option<&str>, device: Option<&str>) -> Result<()> {
    let mut adapter = StrictCliAdapter;
    let result = actions::launch(config, project, device, &mut adapter)?;
    let green = Style::new().green().bold();
    println!(
        "{} Launched '{}' on '{}'.",
        green.apply_to("✓"),
        result.project_name,
        result.device_name
    );
    Ok(())
}

pub fn run(
    config: &Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: bool,
    verbose: bool,
) -> Result<()> {
    let mut adapter = StrictCliAdapter;
    let result = actions::run(
        config,
        project,
        device,
        Some(prebuilt),
        verbose,
        &mut adapter,
    )?;
    let green = Style::new().green().bold();
    println!(
        "{} Running '{}' on '{}'.",
        green.apply_to("✓"),
        result.project_name,
        result.device_name
    );
    Ok(())
}

pub fn sign(
    config: &Config,
    ipa: &str,
    device: Option<&str>,
    identity: Option<&str>,
    profile: Option<&str>,
    launch: bool,
) -> Result<()> {
    let mut adapter = StrictCliAdapter;
    let result = actions::sign_ipa(
        config,
        Path::new(ipa),
        device,
        identity,
        profile,
        launch,
        &mut adapter,
    )?;
    let green = Style::new().green().bold();
    if result.launched {
        println!("{}", green.apply_to("Running!"));
    } else {
        println!("{}", green.apply_to("Installed successfully."));
    }
    Ok(())
}
