use console::Style;
use dialoguer::Select;
use std::path::PathBuf;

use crate::core::config::Config;
use crate::core::device::select_device;
use crate::core::error::{Result, TossError};
use crate::core::project::{
    extract_bundle_id, find_app_in_dir, find_derived_data_build, find_xcode_project,
    list_schemes, resolve_project, select_scheme,
};
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

pub fn install(
    config: &Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: bool,
    verbose: bool,
) -> Result<()> {
    let project_name = resolve_project_arg(config, project)?;
    let bold = Style::new().bold();

    let (app_path, _bundle_id) = if prebuilt {
        resolve_project(config, &project_name)?
    } else {
        // Build from source
        let proj = config.projects.get(&project_name).unwrap();
        let source_path = PathBuf::from(proj.path.as_ref().ok_or_else(|| {
            TossError::Project(format!(
                "project '{}' has no source path — re-register with path or use --prebuilt",
                project_name
            ))
        })?);

        let (project_path, is_workspace) = find_xcode_project(&source_path)?;
        let schemes = list_schemes(&project_path, is_workspace)?;
        let scheme = select_scheme(schemes)?;

        let devices = xcrun::list_devices()?;
        let device_id = select_device(device, config, &devices)?;
        let device_udid = devices
            .iter()
            .find(|d| d.identifier == device_id)
            .map(|d| d.udid.as_str())
            .unwrap_or(&device_id);

        println!("Building {}...", bold.apply_to(&scheme));
        xcrun::build_for_device(&project_path, is_workspace, &scheme, device_udid, verbose)?;

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
        (app_path, bundle_id)
    };

    let devices = xcrun::list_devices()?;
    let device_id = select_device(device, config, &devices)?;

    let device_name = devices
        .iter()
        .find(|d| d.identifier == device_id)
        .map(|d| d.name.as_str())
        .unwrap_or(&device_id);

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

pub fn run(
    config: &Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: bool,
    verbose: bool,
) -> Result<()> {
    let project_name = resolve_project_arg(config, project)?;
    let bold = Style::new().bold();

    let (app_path, bundle_id, device_id) = if prebuilt {
        let (app_path, bundle_id) = resolve_project(config, &project_name)?;
        let devices = xcrun::list_devices()?;
        let device_id = select_device(device, config, &devices)?;
        (app_path, bundle_id, device_id)
    } else {
        // Build from source
        let proj = config.projects.get(&project_name).unwrap();
        let source_path = PathBuf::from(proj.path.as_ref().ok_or_else(|| {
            TossError::Project(format!(
                "project '{}' has no source path — re-register with path or use --prebuilt",
                project_name
            ))
        })?);

        let (project_path, is_workspace) = find_xcode_project(&source_path)?;
        let schemes = list_schemes(&project_path, is_workspace)?;
        let scheme = select_scheme(schemes)?;

        let devices = xcrun::list_devices()?;
        let device_id = select_device(device, config, &devices)?;
        let device_udid = devices
            .iter()
            .find(|d| d.identifier == device_id)
            .map(|d| d.udid.as_str())
            .unwrap_or(&device_id);

        println!("Building {}...", bold.apply_to(&scheme));
        xcrun::build_for_device(&project_path, is_workspace, &scheme, device_udid, verbose)?;

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
        (app_path, bundle_id, device_id)
    };

    let devices = xcrun::list_devices()?;
    let device_name = devices
        .iter()
        .find(|d| d.identifier == device_id)
        .map(|d| d.name.as_str())
        .unwrap_or(&device_id);

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
