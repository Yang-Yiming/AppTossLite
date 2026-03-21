use dialoguer::Select;
use std::path::PathBuf;

use crate::core::config::Config;
use crate::core::error::{Result, TossError};
use crate::core::project::{
    extract_bundle_id, find_app_in_dir, find_derived_data_build, find_xcode_project, list_schemes,
    resolve_project, select_scheme,
};
use crate::core::xcrun;

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
