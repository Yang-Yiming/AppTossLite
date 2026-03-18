use console::Style;
use dialoguer::Select;

use crate::core::actions::{install_app_workflow, launch_app_workflow, run_app_workflow};
use crate::core::config::Config;
use crate::core::error::{Result, TossError};
use crate::core::xcrun;

fn select_project(config: &Config) -> Result<String> {
    let names: Vec<&String> = config.projects.keys().collect();

    if names.is_empty() {
        return Err(TossError::Project(
            "no projects registered — use `toss projects add` or the Projects menu".into(),
        ));
    }

    if names.len() == 1 {
        return Ok(names[0].clone());
    }

    let selection = Select::new()
        .with_prompt("Select project")
        .items(&names)
        .default(0)
        .interact()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    Ok(names[selection].clone())
}

fn select_device(_config: &Config) -> Result<(String, String, String)> {
    use crate::core::device::DeviceState;

    let devices = xcrun::list_devices()?;
    let connected: Vec<_> = devices
        .iter()
        .filter(|d| d.state == DeviceState::Connected)
        .collect();

    if connected.is_empty() {
        return Err(TossError::Device(
            "no connected devices found — plug in a device and try again".into(),
        ));
    }

    if connected.len() == 1 {
        return Ok((
            connected[0].identifier.clone(),
            connected[0].udid.clone(),
            connected[0].name.clone(),
        ));
    }

    let items: Vec<String> = connected
        .iter()
        .map(|d| format!("{} ({})", d.name, d.model))
        .collect();

    let selection = Select::new()
        .with_prompt("Select device")
        .items(&items)
        .default(0)
        .interact()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    Ok((
        connected[selection].identifier.clone(),
        connected[selection].udid.clone(),
        connected[selection].name.clone(),
    ))
}

pub fn install(config: &Config) -> Result<()> {
    let project_name = select_project(config)?;
    let (device_id, device_udid, device_name) = select_device(config)?;
    let bold = Style::new().bold();

    println!(
        "Installing {} → {}...",
        bold.apply_to(&project_name),
        bold.apply_to(&device_name),
    );

    let proj = config.projects.get(&project_name).unwrap();
    let prebuilt = proj.path.is_none();
    install_app_workflow(config, &project_name, &device_id, &device_udid, prebuilt, false)?;

    let green = Style::new().green().bold();
    println!("{}", green.apply_to("Installed successfully."));
    Ok(())
}

pub fn launch(config: &Config) -> Result<()> {
    let project_name = select_project(config)?;
    let (device_id, _device_udid, device_name) = select_device(config)?;

    let bold = Style::new().bold();
    println!(
        "Launching {} on {}...",
        bold.apply_to(&project_name),
        bold.apply_to(&device_name),
    );

    launch_app_workflow(config, &project_name, &device_id)?;

    let green = Style::new().green().bold();
    println!("{}", green.apply_to("Launched successfully."));
    Ok(())
}

pub fn run(config: &Config) -> Result<()> {
    let project_name = select_project(config)?;
    let (device_id, device_udid, device_name) = select_device(config)?;
    let bold = Style::new().bold();

    println!(
        "Installing {} → {}...",
        bold.apply_to(&project_name),
        bold.apply_to(&device_name),
    );

    let proj = config.projects.get(&project_name).unwrap();
    let prebuilt = proj.path.is_none();
    let (_app_path, bundle_id) = run_app_workflow(config, &project_name, &device_id, &device_udid, prebuilt, false)?;

    let green = Style::new().green();
    println!("{}", green.apply_to("Installed."));

    println!("Launching {}...", bold.apply_to(&bundle_id));

    let green_bold = Style::new().green().bold();
    println!("{}", green_bold.apply_to("Running!"));
    Ok(())
}
