use std::path::{Path, PathBuf};

use console::Style;
use dialoguer::Select;

use crate::core::config::Config;
use crate::core::device;
use crate::core::error::{Result, TossError};
use crate::core::project::{
    extract_bundle_id, find_app_in_dir, find_derived_data_build, find_xcode_project, list_schemes,
    resolve_project, select_scheme,
};
use crate::core::sign;
use crate::core::xcrun;

// ---------------------------------------------------------------------------
// Resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a project name: explicit arg → config default → auto/interactive.
pub fn resolve_project_name(config: &Config, project: Option<&str>) -> Result<String> {
    if let Some(name) = project {
        if !config.projects.contains_key(name) {
            return Err(TossError::Project(format!(
                "unknown project '{}' — register it with `toss projects add`",
                name
            )));
        }
        return Ok(name.to_string());
    }

    if let Some(ref default_project) = config.defaults.project {
        if config.projects.contains_key(default_project) {
            return Ok(default_project.clone());
        }
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

/// Resolve a device: returns (identifier, udid, name).
pub fn resolve_device(device: Option<&str>, config: &Config) -> Result<(String, String, String)> {
    let devices = xcrun::list_devices()?;
    let device_id = device::select_device(device, config, &devices)?;
    let dev = devices.iter().find(|d| d.identifier == device_id);
    Ok((
        device_id.clone(),
        dev.map(|d| d.udid.clone())
            .unwrap_or_else(|| device_id.clone()),
        dev.map(|d| d.name.clone()).unwrap_or(device_id),
    ))
}

// ---------------------------------------------------------------------------
// Low-level workflows (no output, no resolution)
// ---------------------------------------------------------------------------

pub fn install_app_workflow(
    config: &Config,
    project_name: &str,
    device_id: &str,
    device_udid: &str,
    prebuilt: bool,
    verbose: bool,
) -> Result<PathBuf> {
    let app_path = if prebuilt {
        let (app_path, _bundle_id) = resolve_project(config, project_name)?;
        app_path
    } else {
        let proj = config.projects.get(project_name).unwrap();
        let source_path = PathBuf::from(proj.path.as_ref().ok_or_else(|| {
            TossError::Project(format!(
                "project '{}' has no source path — re-register with path or use --prebuilt",
                project_name
            ))
        })?);

        let (project_path, is_workspace) = find_xcode_project(&source_path)?;
        let schemes = list_schemes(&project_path, is_workspace)?;
        let scheme = select_scheme(schemes)?;

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
        build_dir.join(&app_name)
    };

    xcrun::install_app(device_id, &app_path)?;
    Ok(app_path)
}

pub fn launch_app_workflow(config: &Config, project_name: &str, device_id: &str) -> Result<String> {
    let (_app_path, bundle_id) = resolve_project(config, project_name)?;
    xcrun::launch_app(device_id, &bundle_id)?;
    Ok(bundle_id)
}

pub fn run_app_workflow(
    config: &Config,
    project_name: &str,
    device_id: &str,
    device_udid: &str,
    prebuilt: bool,
    verbose: bool,
) -> Result<(PathBuf, String)> {
    let (app_path, bundle_id) = if prebuilt {
        resolve_project(config, project_name)?
    } else {
        let proj = config.projects.get(project_name).unwrap();
        let source_path = PathBuf::from(proj.path.as_ref().ok_or_else(|| {
            TossError::Project(format!(
                "project '{}' has no source path — re-register with path or use --prebuilt",
                project_name
            ))
        })?);

        let (project_path, is_workspace) = find_xcode_project(&source_path)?;
        let schemes = list_schemes(&project_path, is_workspace)?;
        let scheme = select_scheme(schemes)?;

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

    xcrun::install_app(device_id, &app_path)?;
    xcrun::launch_app(device_id, &bundle_id)?;
    Ok((app_path, bundle_id))
}

// ---------------------------------------------------------------------------
// High-level commands (resolve + output + workflow)
// ---------------------------------------------------------------------------

/// Auto-detect prebuilt from project config when not explicitly specified.
fn resolve_prebuilt(config: &Config, project_name: &str, prebuilt: Option<bool>) -> bool {
    prebuilt.unwrap_or_else(|| {
        config
            .projects
            .get(project_name)
            .map(|p| p.path.is_none())
            .unwrap_or(true)
    })
}

pub fn install(
    config: &Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: Option<bool>,
    verbose: bool,
) -> Result<()> {
    let project_name = resolve_project_name(config, project)?;
    let (device_id, device_udid, device_name) = resolve_device(device, config)?;
    let prebuilt = resolve_prebuilt(config, &project_name, prebuilt);

    let bold = Style::new().bold();
    println!(
        "Installing {} → {}...",
        bold.apply_to(&project_name),
        bold.apply_to(&device_name),
    );

    install_app_workflow(
        config,
        &project_name,
        &device_id,
        &device_udid,
        prebuilt,
        verbose,
    )?;

    let green = Style::new().green().bold();
    println!("{}", green.apply_to("Installed successfully."));
    Ok(())
}

pub fn launch(config: &Config, project: Option<&str>, device: Option<&str>) -> Result<()> {
    let project_name = resolve_project_name(config, project)?;
    let (device_id, _device_udid, device_name) = resolve_device(device, config)?;

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

pub fn run(
    config: &Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: Option<bool>,
    verbose: bool,
) -> Result<()> {
    let project_name = resolve_project_name(config, project)?;
    let (device_id, device_udid, device_name) = resolve_device(device, config)?;
    let prebuilt = resolve_prebuilt(config, &project_name, prebuilt);

    let bold = Style::new().bold();
    println!(
        "Installing {} → {}...",
        bold.apply_to(&project_name),
        bold.apply_to(&device_name),
    );

    let (_app_path, bundle_id) = run_app_workflow(
        config,
        &project_name,
        &device_id,
        &device_udid,
        prebuilt,
        verbose,
    )?;

    let green = Style::new().green();
    println!("{}", green.apply_to("Installed."));
    println!("Launching {}...", bold.apply_to(&bundle_id));

    let green_bold = Style::new().green().bold();
    println!("{}", green_bold.apply_to("Running!"));
    Ok(())
}

pub fn sign_ipa(
    config: &Config,
    ipa: &Path,
    device: Option<&str>,
    identity: Option<&str>,
    profile: Option<&str>,
    launch: bool,
) -> Result<()> {
    let (device_id, _device_udid, device_name) = resolve_device(device, config)?;

    let bold = Style::new().bold();
    let ipa_name = ipa
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ipa.to_string_lossy().to_string());
    println!(
        "Signing {} → {}...",
        bold.apply_to(&ipa_name),
        bold.apply_to(&device_name),
    );

    sign::sign_workflow(config, ipa, &device_id, identity, profile, launch)?;

    let green = Style::new().green().bold();
    if launch {
        println!("{}", green.apply_to("Running!"));
    } else {
        println!("{}", green.apply_to("Installed successfully."));
    }
    Ok(())
}
