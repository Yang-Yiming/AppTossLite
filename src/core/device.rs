use super::config::Config;
use super::error::{Result, TossError};

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub identifier: String,
    pub model: String,
    pub os_version: String,
    pub state: DeviceState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceState {
    Connected,
    Disconnected,
    Unknown(String),
}

impl std::fmt::Display for DeviceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceState::Connected => write!(f, "connected"),
            DeviceState::Disconnected => write!(f, "disconnected"),
            DeviceState::Unknown(s) => write!(f, "{}", s),
        }
    }
}

/// Resolve a device identifier: could be an alias, UDID, or index into the device list.
pub fn resolve_device_id(identifier: &str, config: &Config, devices: &[Device]) -> Result<String> {
    // Check alias first
    if let Some(udid) = config.devices.aliases.get(identifier) {
        return Ok(udid.clone());
    }

    // Check if it's a UDID directly matching a device
    if devices.iter().any(|d| d.identifier == identifier) {
        return Ok(identifier.to_string());
    }

    // Check if it's a numeric index (1-based)
    if let Ok(idx) = identifier.parse::<usize>() {
        if idx >= 1 && idx <= devices.len() {
            return Ok(devices[idx - 1].identifier.clone());
        }
        return Err(TossError::Device(format!(
            "device index {} out of range (1-{})",
            idx,
            devices.len()
        )));
    }

    Err(TossError::Device(format!(
        "unknown device '{}' — not an alias, UDID, or valid index",
        identifier
    )))
}

/// Select a device: auto if one connected, prompt if multiple, error if none.
pub fn select_device(
    device_flag: Option<&str>,
    config: &Config,
    devices: &[Device],
) -> Result<String> {
    // If user specified a device, resolve it
    if let Some(id) = device_flag {
        return resolve_device_id(id, config, devices);
    }

    let connected: Vec<&Device> = devices
        .iter()
        .filter(|d| d.state == DeviceState::Connected)
        .collect();

    match connected.len() {
        0 => Err(TossError::Device(
            "no connected devices found — plug in a device and try again".into(),
        )),
        1 => Ok(connected[0].identifier.clone()),
        _ => {
            // Interactive prompt
            let items: Vec<String> = connected
                .iter()
                .map(|d| format!("{} ({})", d.name, d.model))
                .collect();

            let selection = dialoguer::Select::new()
                .with_prompt("Multiple devices connected — choose one")
                .items(&items)
                .default(0)
                .interact()
                .map_err(|e| TossError::UserCancelled(e.to_string()))?;

            Ok(connected[selection].identifier.clone())
        }
    }
}
