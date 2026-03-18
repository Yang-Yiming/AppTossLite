use console::Style;
use dialoguer::Select;
use std::path::PathBuf;

use crate::core::config::Config;
use crate::core::error::{Result, TossError};
use crate::core::project::{
    extract_bundle_id, find_app_in_dir, find_derived_data_build, find_xcode_project,
    list_schemes, select_scheme,
};
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

fn select_device(_config: &Config) -> Result<(String, String)> {
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
        return Ok((connected[0].identifier.clone(), connected[0].name.clone()));
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
        connected[selection].name.clone(),
    ))
}

pub fn install(config: &Config) -> Result<()> {
    let project_name = select_project(config)?;
    let proj = config.projects.get(&project_name).unwrap();
    let bold = Style::new().bold();

    let app_path = if let Some(path) = &proj.path {
        // Build from source
        let source_path = PathBuf::from(path);
        let (project_path, is_workspace) = find_xcode_project(&source_path)?;
        let schemes = list_schemes(&project_path, is_workspace)?;
        let scheme = select_scheme(schemes)?;
        let (device_id, _device_name) = select_device(config)?;

        println!("Building {}...", bold.apply_to(&scheme));
        xcrun::build_for_device(&project_path, is_workspace, &scheme, &device_id)?;

        let build_dirs = find_derived_data_build(&source_path)?;
        let build_dir = if build_dirs.len() == 1 {
            &build_dirs[0]
        } else {
            let items: Vec<String> = build_dirs.iter().map(|p| p.display().to_string()).collect();
            let selection = Select::new()
                .with_prompt("Multiple build outputs found, select one")
                .items(&items)
                .default(0)
                .interact()
                .map_err(|e| TossError::UserCancelled(e.to_string()))?;
            &build_dirs[selection]
        };

        let app_name = find_app_in_dir(build_dir)?;
        build_dir.join(&app_name)
    } else {
        // Use prebuilt
        use crate::core::project::resolve_project;
        let (app_path, _) = resolve_project(config, &project_name)?;
        app_path
    };

    let (device_id, device_name) = select_device(config)?;

    println!(
        "Installing {} → {}...",
        bold.apply_to(app_path.file_name().unwrap().to_string_lossy()),
        bold.apply_to(&device_name),
    );

    xcrun::install_app(&device_id, &app_path)?;

    let green = Style::new().green().bold();
    println!("{}", green.apply_to("Installed successfully."));
    Ok(())
}

pub fn launch(config: &Config) -> Result<()> {
    use crate::core::project::resolve_project;

    let project_name = select_project(config)?;
    let (_app_path, bundle_id) = resolve_project(config, &project_name)?;
    let (device_id, device_name) = select_device(config)?;

    let bold = Style::new().bold();
    println!(
        "Launching {} on {}...",
        bold.apply_to(&bundle_id),
        bold.apply_to(&device_name),
    );

    xcrun::launch_app(&device_id, &bundle_id)?;

    let green = Style::new().green().bold();
    println!("{}", green.apply_to("Launched successfully."));
    Ok(())
}

pub fn run(config: &Config) -> Result<()> {
    let project_name = select_project(config)?;
    let proj = config.projects.get(&project_name).unwrap();
    let bold = Style::new().bold();

    let (app_path, bundle_id, device_id, device_name) = if let Some(path) = &proj.path {
        // Build from source
        let source_path = PathBuf::from(path);
        let (project_path, is_workspace) = find_xcode_project(&source_path)?;
        let schemes = list_schemes(&project_path, is_workspace)?;
        let scheme = select_scheme(schemes)?;
        let (device_id, device_name) = select_device(config)?;

        println!("Building {}...", bold.apply_to(&scheme));
        xcrun::build_for_device(&project_path, is_workspace, &scheme, &device_id)?;

        let build_dirs = find_derived_data_build(&source_path)?;
        let build_dir = if build_dirs.len() == 1 {
            &build_dirs[0]
        } else {
            let items: Vec<String> = build_dirs.iter().map(|p| p.display().to_string()).collect();
            let selection = Select::new()
                .with_prompt("Multiple build outputs found, select one")
                .items(&items)
                .default(0)
                .interact()
                .map_err(|e| TossError::UserCancelled(e.to_string()))?;
            &build_dirs[selection]
        };

        let app_name = find_app_in_dir(build_dir)?;
        let app_path = build_dir.join(&app_name);
        let bundle_id = extract_bundle_id(&app_path)?;
        (app_path, bundle_id, device_id, device_name)
    } else {
        // Use prebuilt
        use crate::core::project::resolve_project;
        let (app_path, bundle_id) = resolve_project(config, &project_name)?;
        let (device_id, device_name) = select_device(config)?;
        (app_path, bundle_id, device_id, device_name)
    };

    println!(
        "Installing {} → {}...",
        bold.apply_to(app_path.file_name().unwrap().to_string_lossy()),
        bold.apply_to(&device_name),
    );
    xcrun::install_app(&device_id, &app_path)?;

    let green = Style::new().green();
    println!("{}", green.apply_to("Installed."));

    println!("Launching {}...", bold.apply_to(&bundle_id));
    xcrun::launch_app(&device_id, &bundle_id)?;

    let green_bold = Style::new().green().bold();
    println!("{}", green_bold.apply_to("Running!"));
    Ok(())
}
