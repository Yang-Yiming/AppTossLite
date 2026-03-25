use std::path::{Path, PathBuf};

use crate::core::config::Config;
use crate::core::device;
use crate::core::error::{Result, TossError};
use crate::core::interaction::{WorkflowAdapter, WorkflowEvent, choose_index};
use crate::core::project::{
    extract_bundle_id, find_app_in_dir, find_derived_data_build, find_xcode_project, list_schemes,
    resolve_project, select_scheme,
};
use crate::core::sign;
use crate::core::time;
use crate::core::xcrun;

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub project_name: String,
    pub device_id: String,
    pub device_udid: String,
    pub device_name: String,
    pub app_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LaunchResult {
    pub project_name: String,
    pub device_id: String,
    pub device_name: String,
    pub bundle_id: String,
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub project_name: String,
    pub device_id: String,
    pub device_udid: String,
    pub device_name: String,
    pub app_path: PathBuf,
    pub bundle_id: String,
}

#[derive(Debug, Clone)]
pub struct SignResult {
    pub device_id: String,
    pub device_udid: String,
    pub device_name: String,
    pub app_path: PathBuf,
    pub final_bundle_id: String,
    pub launched: bool,
}

pub fn resolve_project_name(
    config: &Config,
    project: Option<&str>,
    adapter: &mut impl WorkflowAdapter,
) -> Result<String> {
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
        adapter.emit(WorkflowEvent::Warning {
            message: format!(
                "default project '{}' not found in config, ignoring",
                default_project
            ),
        })?;
    }

    let names: Vec<String> = config.projects.keys().cloned().collect();

    match names.len() {
        0 => Err(TossError::Project(
            "no projects registered — use `toss projects add` to register one".into(),
        )),
        1 => Ok(names[0].clone()),
        _ => {
            let selection = choose_index(
                adapter,
                "Select project",
                &names,
                TossError::Project(
                    "multiple projects registered — specify one as a CLI argument".into(),
                ),
            )?;
            Ok(names[selection].clone())
        }
    }
}

pub fn resolve_device(
    device: Option<&str>,
    config: &Config,
    adapter: &mut impl WorkflowAdapter,
) -> Result<(String, String, String)> {
    let devices = xcrun::list_devices()?;
    let device_id = device::select_device(device, config, &devices, adapter)?;
    let dev = devices.iter().find(|d| d.identifier == device_id);
    Ok((
        device_id.clone(),
        dev.map(|d| d.udid.clone())
            .unwrap_or_else(|| device_id.clone()),
        dev.map(|d| d.name.clone()).unwrap_or(device_id),
    ))
}

pub fn install_app_workflow(
    config: &Config,
    project_name: &str,
    device_id: &str,
    device_udid: &str,
    device_name: &str,
    prebuilt: bool,
    verbose: bool,
    adapter: &mut impl WorkflowAdapter,
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
        let scheme = select_scheme(schemes, adapter)?;

        adapter.emit(WorkflowEvent::Building {
            project: project_name.to_string(),
            scheme: scheme.clone(),
            device_udid: device_udid.to_string(),
        })?;
        build_for_install_or_run(
            &project_path,
            is_workspace,
            &scheme,
            device_udid,
            verbose,
            adapter,
        )?;
        adapter.emit(WorkflowEvent::BuildSucceeded)?;

        let build_dirs = find_derived_data_build(&source_path)?;
        let build_dir = if build_dirs.len() == 1 {
            &build_dirs[0]
        } else {
            let items: Vec<String> = build_dirs.iter().map(|p| p.display().to_string()).collect();
            let selection = choose_index(
                adapter,
                "Multiple build outputs found, select one",
                &items,
                TossError::Project(
                    "multiple build outputs found — use the TUI to choose one".into(),
                ),
            )?;
            &build_dirs[selection]
        };

        let app_name = find_app_in_dir(build_dir)?;
        build_dir.join(&app_name)
    };

    install_app_with_fallback(device_id, device_udid, device_name, &app_path, adapter)?;
    Ok(app_path)
}

pub fn launch_app_workflow(
    config: &Config,
    project_name: &str,
    device_id: &str,
    device_udid: &str,
    device_name: &str,
    adapter: &mut impl WorkflowAdapter,
) -> Result<String> {
    let (_app_path, bundle_id) = resolve_project(config, project_name)?;
    launch_app_with_fallback(device_id, device_udid, device_name, &bundle_id, adapter)?;
    Ok(bundle_id)
}

pub fn run_app_workflow(
    config: &Config,
    project_name: &str,
    device_id: &str,
    device_udid: &str,
    device_name: &str,
    prebuilt: bool,
    verbose: bool,
    adapter: &mut impl WorkflowAdapter,
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
        let scheme = select_scheme(schemes, adapter)?;

        adapter.emit(WorkflowEvent::Building {
            project: project_name.to_string(),
            scheme: scheme.clone(),
            device_udid: device_udid.to_string(),
        })?;
        build_for_install_or_run(
            &project_path,
            is_workspace,
            &scheme,
            device_udid,
            verbose,
            adapter,
        )?;
        adapter.emit(WorkflowEvent::BuildSucceeded)?;

        let build_dirs = find_derived_data_build(&source_path)?;
        let build_dir = if build_dirs.len() == 1 {
            &build_dirs[0]
        } else {
            let items: Vec<String> = build_dirs.iter().map(|p| p.display().to_string()).collect();
            let selection = choose_index(
                adapter,
                "Multiple build outputs found, select one",
                &items,
                TossError::Project(
                    "multiple build outputs found — use the TUI to choose one".into(),
                ),
            )?;
            &build_dirs[selection]
        };

        let app_name = find_app_in_dir(build_dir)?;
        let app_path = build_dir.join(&app_name);
        let bundle_id = extract_bundle_id(&app_path)?;
        (app_path, bundle_id)
    };

    install_app_with_fallback(device_id, device_udid, device_name, &app_path, adapter)?;
    launch_app_with_fallback(device_id, device_udid, device_name, &bundle_id, adapter)?;
    Ok((app_path, bundle_id))
}

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
    config: &mut Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: Option<bool>,
    verbose: bool,
    adapter: &mut impl WorkflowAdapter,
) -> Result<InstallResult> {
    let project_name = resolve_project_name(config, project, adapter)?;
    let (device_id, device_udid, device_name) = resolve_device(device, config, adapter)?;
    let prebuilt = resolve_prebuilt(config, &project_name, prebuilt);

    let app_path = install_app_workflow(
        config,
        &project_name,
        &device_id,
        &device_udid,
        &device_name,
        prebuilt,
        verbose,
        adapter,
    )?;

    adapter.emit(WorkflowEvent::Installing {
        app_path: app_path.clone(),
        device_name: device_name.clone(),
    })?;
    record_project_tossed(config, &project_name)?;

    Ok(InstallResult {
        project_name,
        device_id,
        device_udid,
        device_name,
        app_path,
    })
}

pub fn launch(
    config: &Config,
    project: Option<&str>,
    device: Option<&str>,
    adapter: &mut impl WorkflowAdapter,
) -> Result<LaunchResult> {
    let project_name = resolve_project_name(config, project, adapter)?;
    let (device_id, _device_udid, device_name) = resolve_device(device, config, adapter)?;

    let bundle_id = launch_app_workflow(
        config,
        &project_name,
        &device_id,
        &_device_udid,
        &device_name,
        adapter,
    )?;
    adapter.emit(WorkflowEvent::Launching {
        bundle_id: bundle_id.clone(),
        device_name: device_name.clone(),
    })?;

    Ok(LaunchResult {
        project_name,
        device_id,
        device_name,
        bundle_id,
    })
}

pub fn run(
    config: &mut Config,
    project: Option<&str>,
    device: Option<&str>,
    prebuilt: Option<bool>,
    verbose: bool,
    adapter: &mut impl WorkflowAdapter,
) -> Result<RunResult> {
    let project_name = resolve_project_name(config, project, adapter)?;
    let (device_id, device_udid, device_name) = resolve_device(device, config, adapter)?;
    let prebuilt = resolve_prebuilt(config, &project_name, prebuilt);

    let (app_path, bundle_id) = run_app_workflow(
        config,
        &project_name,
        &device_id,
        &device_udid,
        &device_name,
        prebuilt,
        verbose,
        adapter,
    )?;

    adapter.emit(WorkflowEvent::Installing {
        app_path: app_path.clone(),
        device_name: device_name.clone(),
    })?;
    record_project_tossed(config, &project_name)?;
    adapter.emit(WorkflowEvent::Launching {
        bundle_id: bundle_id.clone(),
        device_name: device_name.clone(),
    })?;

    Ok(RunResult {
        project_name,
        device_id,
        device_udid,
        device_name,
        app_path,
        bundle_id,
    })
}

fn record_project_tossed(config: &mut Config, project_name: &str) -> Result<()> {
    let timestamp = time::now_rfc3339()?;
    let project = config.projects.get_mut(project_name).ok_or_else(|| {
        TossError::Project(format!(
            "unknown project '{}' while updating last toss time",
            project_name
        ))
    })?;
    project.last_tossed_at = Some(timestamp);
    config.save()
}

fn build_for_install_or_run(
    project_path: &Path,
    is_workspace: bool,
    scheme: &str,
    device_udid: &str,
    verbose: bool,
    adapter: &mut impl WorkflowAdapter,
) -> Result<()> {
    match xcrun::build_for_device(project_path, is_workspace, scheme, device_udid, verbose) {
        Ok(()) => Ok(()),
        Err(err) if should_retry_generic_ios_build(&err) => {
            adapter.emit(WorkflowEvent::Warning {
                message: format!(
                    "device-specific xcodebuild destination was unavailable for '{}' — retrying with generic iOS destination",
                    device_udid
                ),
            })?;
            xcrun::build_for_generic_ios(project_path, is_workspace, scheme, verbose)
        }
        Err(err) => Err(err),
    }
}

fn should_retry_generic_ios_build(err: &TossError) -> bool {
    match err {
        TossError::Xcrun(message) => {
            message.contains(
                "Unable to find a destination matching the provided destination specifier",
            ) || message
                .contains("Supported platforms for the buildables in the current scheme is empty")
        }
        _ => false,
    }
}

fn install_app_with_fallback(
    device_id: &str,
    device_udid: &str,
    device_name: &str,
    app_path: &Path,
    adapter: &mut impl WorkflowAdapter,
) -> Result<()> {
    match xcrun::install_app(device_id, app_path) {
        Ok(()) => Ok(()),
        Err(err) if should_retry_devicectl_identifier(&err) => {
            adapter.emit(WorkflowEvent::Warning {
                message: format!(
                    "devicectl could not locate device identifier '{}' — retrying install with hardware UDID",
                    device_id
                ),
            })?;
            retry_install_with_udid_or_name(device_udid, device_name, app_path, adapter)
        }
        Err(err) => Err(err),
    }
}

fn launch_app_with_fallback(
    device_id: &str,
    device_udid: &str,
    device_name: &str,
    bundle_id: &str,
    adapter: &mut impl WorkflowAdapter,
) -> Result<()> {
    match xcrun::launch_app(device_id, bundle_id) {
        Ok(()) => Ok(()),
        Err(err) if should_retry_devicectl_identifier(&err) => {
            adapter.emit(WorkflowEvent::Warning {
                message: format!(
                    "devicectl could not locate device identifier '{}' — retrying launch with hardware UDID",
                    device_id
                ),
            })?;
            retry_launch_with_udid_or_name(device_udid, device_name, bundle_id, adapter)
        }
        Err(err) => Err(err),
    }
}

fn should_retry_devicectl_identifier(err: &TossError) -> bool {
    match err {
        TossError::Xcrun(message) => {
            message.contains("CoreDeviceService was unable to locate a device matching the requested device identifier")
                || message.contains("com.apple.dt.CoreDeviceError error 1011")
        }
        _ => false,
    }
}

fn retry_install_with_udid_or_name(
    device_udid: &str,
    device_name: &str,
    app_path: &Path,
    adapter: &mut impl WorkflowAdapter,
) -> Result<()> {
    match xcrun::install_app(device_udid, app_path) {
        Ok(()) => Ok(()),
        Err(err) if should_retry_devicectl_identifier(&err) && !device_name.is_empty() => {
            adapter.emit(WorkflowEvent::Warning {
                message: format!(
                    "devicectl could not locate hardware UDID '{}' — retrying install with device name '{}'",
                    device_udid, device_name
                ),
            })?;
            xcrun::install_app(device_name, app_path)
        }
        Err(err) => Err(err),
    }
}

fn retry_launch_with_udid_or_name(
    device_udid: &str,
    device_name: &str,
    bundle_id: &str,
    adapter: &mut impl WorkflowAdapter,
) -> Result<()> {
    match xcrun::launch_app(device_udid, bundle_id) {
        Ok(()) => Ok(()),
        Err(err) if should_retry_devicectl_identifier(&err) && !device_name.is_empty() => {
            adapter.emit(WorkflowEvent::Warning {
                message: format!(
                    "devicectl could not locate hardware UDID '{}' — retrying launch with device name '{}'",
                    device_udid, device_name
                ),
            })?;
            xcrun::launch_app(device_name, bundle_id)
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_generic_build_for_missing_destination_errors() {
        let err = TossError::Xcrun(
            "xcodebuild failed:\nUnable to find a destination matching the provided destination specifier"
                .into(),
        );

        assert!(should_retry_generic_ios_build(&err));
    }

    #[test]
    fn does_not_retry_generic_build_for_other_errors() {
        let err = TossError::Xcrun("xcodebuild failed:\nCode signing failed".into());

        assert!(!should_retry_generic_ios_build(&err));
    }

    #[test]
    fn retries_devicectl_identifier_for_missing_coredevice_identifier() {
        let err = TossError::Xcrun(
            "install failed:\nERROR: CoreDeviceService was unable to locate a device matching the requested device identifier. (com.apple.dt.CoreDeviceError error 1011)".into(),
        );

        assert!(should_retry_devicectl_identifier(&err));
    }

    #[test]
    fn does_not_retry_devicectl_identifier_for_other_errors() {
        let err = TossError::Xcrun("install failed:\npermission denied".into());

        assert!(!should_retry_devicectl_identifier(&err));
    }
}

pub fn sign_ipa(
    config: &Config,
    ipa: &Path,
    device: Option<&str>,
    identity: Option<&str>,
    profile: Option<&str>,
    launch: bool,
    adapter: &mut impl WorkflowAdapter,
) -> Result<SignResult> {
    let (device_id, device_udid, device_name) = resolve_device(device, config, adapter)?;

    adapter.emit(WorkflowEvent::Signing {
        ipa_name: ipa
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| ipa.to_string_lossy().to_string()),
        device_name: device_name.clone(),
    })?;

    sign::sign_workflow(
        config,
        ipa,
        &device_id,
        &device_udid,
        identity,
        profile,
        launch,
        adapter,
    )
    .map(|result| SignResult {
        device_id,
        device_udid,
        device_name,
        app_path: result.app_path,
        final_bundle_id: result.final_bundle_id,
        launched: result.launched,
    })
}
