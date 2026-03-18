use console::Style;

use crate::core::config::Config;
use crate::core::device::select_device;
use crate::core::error::Result;
use crate::core::project::resolve_project;
use crate::core::xcrun;

pub fn install(config: &Config, project: &str, device: Option<&str>) -> Result<()> {
    let (app_path, _bundle_id) = resolve_project(config, project)?;

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

pub fn launch(config: &Config, project: &str, device: Option<&str>) -> Result<()> {
    let (_app_path, bundle_id) = resolve_project(config, project)?;

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

pub fn run(config: &Config, project: &str, device: Option<&str>) -> Result<()> {
    let (app_path, bundle_id) = resolve_project(config, project)?;

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
