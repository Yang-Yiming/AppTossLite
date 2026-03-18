use console::Style;
use dialoguer::Select;

use crate::core::config::Config;
use crate::core::device::select_device;
use crate::core::error::{Result, TossError};
use crate::core::project::resolve_project;
use crate::core::xcrun;

/// Resolve a project argument using the fallback chain:
/// 1. Explicit argument
/// 2. defaults.project from config
/// 3. Only one registered project → use it
/// 4. Multiple → interactive prompt
/// 5. None → error
fn resolve_project_arg(config: &Config, project: Option<&str>) -> Result<String> {
    // Explicit argument
    if let Some(name) = project {
        if !config.projects.contains_key(name) {
            return Err(TossError::Project(format!(
                "unknown project '{}' — register it with `toss projects add`",
                name
            )));
        }
        return Ok(name.to_string());
    }

    // Default from config
    if let Some(ref default_project) = config.defaults.project {
        if config.projects.contains_key(default_project) {
            return Ok(default_project.clone());
        }
        // Default is set but project no longer exists — warn and fall through
        eprintln!(
            "warning: default project '{}' not found in config, ignoring",
            default_project
        );
    }

    let names: Vec<&String> = config.projects.keys().collect();

    match names.len() {
        0 => Err(TossError::Project(
            "no projects registered — use `toss projects add` to register one".into(),
        )),
        1 => Ok(names[0].clone()),
        _ => {
            let selection = Select::new()
                .with_prompt("Select project")
                .items(&names)
                .default(0)
                .interact()
                .map_err(|e| TossError::UserCancelled(e.to_string()))?;
            Ok(names[selection].clone())
        }
    }
}

pub fn install(config: &Config, project: Option<&str>, device: Option<&str>) -> Result<()> {
    let project_name = resolve_project_arg(config, project)?;
    let (app_path, _bundle_id) = resolve_project(config, &project_name)?;

    let devices = xcrun::list_devices()?;
    let device_id = select_device(device, config, &devices)?;

    let device_name = devices
        .iter()
        .find(|d| d.identifier == device_id)
        .map(|d| d.name.as_str())
        .unwrap_or(&device_id);

    let bold = Style::new().bold();
    println!(
        "Installing {} → {}...",
        bold.apply_to(app_path.file_name().unwrap().to_string_lossy()),
        bold.apply_to(device_name),
    );

    xcrun::install_app(&device_id, &app_path)?;

    let green = Style::new().green().bold();
    println!("{}", green.apply_to("Installed successfully."));
    Ok(())
}

pub fn launch(config: &Config, project: Option<&str>, device: Option<&str>) -> Result<()> {
    let project_name = resolve_project_arg(config, project)?;
    let (_app_path, bundle_id) = resolve_project(config, &project_name)?;

    let devices = xcrun::list_devices()?;
    let device_id = select_device(device, config, &devices)?;

    let device_name = devices
        .iter()
        .find(|d| d.identifier == device_id)
        .map(|d| d.name.as_str())
        .unwrap_or(&device_id);

    let bold = Style::new().bold();
    println!(
        "Launching {} on {}...",
        bold.apply_to(&bundle_id),
        bold.apply_to(device_name),
    );

    xcrun::launch_app(&device_id, &bundle_id)?;

    let green = Style::new().green().bold();
    println!("{}", green.apply_to("Launched successfully."));
    Ok(())
}

pub fn run(config: &Config, project: Option<&str>, device: Option<&str>) -> Result<()> {
    let project_name = resolve_project_arg(config, project)?;
    let (app_path, bundle_id) = resolve_project(config, &project_name)?;

    let devices = xcrun::list_devices()?;
    let device_id = select_device(device, config, &devices)?;

    let device_name = devices
        .iter()
        .find(|d| d.identifier == device_id)
        .map(|d| d.name.as_str())
        .unwrap_or(&device_id);

    let bold = Style::new().bold();

    // Install
    println!(
        "Installing {} → {}...",
        bold.apply_to(app_path.file_name().unwrap().to_string_lossy()),
        bold.apply_to(device_name),
    );
    xcrun::install_app(&device_id, &app_path)?;

    let green = Style::new().green();
    println!("{}", green.apply_to("Installed."));

    // Launch
    println!("Launching {}...", bold.apply_to(&bundle_id),);
    xcrun::launch_app(&device_id, &bundle_id)?;

    let green_bold = Style::new().green().bold();
    println!("{}", green_bold.apply_to("Running!"));
    Ok(())
}
