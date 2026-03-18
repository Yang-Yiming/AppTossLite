use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use tempfile::NamedTempFile;

use super::device::{Device, DeviceState};
use super::error::{Result, TossError};

/// Raw JSON structures from `xcrun devicectl list devices --json-output`
#[derive(Debug, Deserialize)]
struct DeviceCtlOutput {
    result: DeviceCtlResult,
}

#[derive(Debug, Deserialize)]
struct DeviceCtlResult {
    devices: Vec<DeviceCtlDevice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceCtlDevice {
    #[serde(default)]
    device_properties: DeviceProperties,
    #[serde(default)]
    hardware_properties: HardwareProperties,
    identifier: String,
    connection_properties: ConnectionProperties,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceProperties {
    #[serde(default)]
    name: String,
    #[serde(default)]
    os_version_number: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HardwareProperties {
    #[serde(default)]
    product_type: String,
    #[serde(default)]
    marketing_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionProperties {
    #[serde(default)]
    tunnel_state: String,
}

pub fn list_devices() -> Result<Vec<Device>> {
    let tmp = NamedTempFile::new()?;
    let tmp_path = tmp.path().to_string_lossy().to_string();

    let output = Command::new("xcrun")
        .args(["devicectl", "list", "devices", "--json-output", &tmp_path])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TossError::Xcrun(format!(
            "devicectl list devices failed: {}",
            stderr.trim()
        )));
    }

    let json_content = std::fs::read_to_string(tmp.path())?;
    let parsed: DeviceCtlOutput = serde_json::from_str(&json_content)?;

    let devices = parsed
        .result
        .devices
        .into_iter()
        .map(|d| {
            let state = match d.connection_properties.tunnel_state.as_str() {
                "connected" => DeviceState::Connected,
                "disconnected" => DeviceState::Disconnected,
                other => DeviceState::Unknown(other.to_string()),
            };
            let model = d
                .hardware_properties
                .marketing_name
                .unwrap_or(d.hardware_properties.product_type);
            Device {
                name: d.device_properties.name,
                identifier: d.identifier,
                model,
                os_version: d.device_properties.os_version_number,
                state,
            }
        })
        .collect();

    Ok(devices)
}

pub fn install_app(device_id: &str, app_path: &Path) -> Result<()> {
    if !app_path.exists() {
        return Err(TossError::Xcrun(format!(
            "app bundle not found at {}",
            app_path.display()
        )));
    }

    let output = Command::new("xcrun")
        .args([
            "devicectl",
            "device",
            "install",
            "app",
            "--device",
            device_id,
            &app_path.to_string_lossy(),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(TossError::Xcrun(format!(
            "install failed:\n{}{}",
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", stdout.trim())
            }
        )));
    }

    Ok(())
}

pub fn launch_app(device_id: &str, bundle_id: &str) -> Result<()> {
    let output = Command::new("xcrun")
        .args([
            "devicectl",
            "device",
            "process",
            "launch",
            "--device",
            device_id,
            bundle_id,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(TossError::Xcrun(format!(
            "launch failed:\n{}{}",
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", stdout.trim())
            }
        )));
    }

    Ok(())
}
