use std::path::Path;

use console::Style;

use crate::cli::adapters::StrictCliAdapter;
use crate::cli::signing;
use crate::core::actions;
use crate::core::config::Config;
use crate::core::error::Result;

pub fn install(
    config: &mut Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: bool,
    verbose: bool,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        let mut adapter = StrictCliAdapter;
        let project_name = actions::resolve_project_name(config, project, &mut adapter)?;
        return signing::preview_project_install(config, &project_name, device, prebuilt, false);
    }
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
    config: &mut Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: bool,
    verbose: bool,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        let mut adapter = StrictCliAdapter;
        let project_name = actions::resolve_project_name(config, project, &mut adapter)?;
        return signing::preview_project_install(config, &project_name, device, prebuilt, true);
    }
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
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        return signing::preview_ipa_install(
            config,
            Path::new(ipa),
            device,
            identity,
            profile,
        );
    }
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
