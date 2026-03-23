use super::config::Config;
use super::error::{Result, TossError};
use super::interaction::{WorkflowAdapter, WorkflowEvent, choose_index};

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub identifier: String,
    pub udid: String,
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

#[derive(Debug, Clone)]
pub struct DeviceAliasResult {
    pub alias: String,
    pub udid: String,
    pub device_name: String,
    pub is_default: bool,
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

/// Select a device: explicit flag → default → auto if one connected → prompt if multiple → error.
pub fn select_device(
    device_flag: Option<&str>,
    config: &Config,
    devices: &[Device],
    adapter: &mut impl WorkflowAdapter,
) -> Result<String> {
    // If user specified a device, resolve it
    if let Some(id) = device_flag {
        return resolve_device_id(id, config, devices);
    }

    // Try the configured default device
    if let Some(ref default_device) = config.defaults.device {
        return match resolve_device_id(default_device, config, devices) {
            Ok(id) => Ok(id),
            Err(_) => {
                adapter.emit(WorkflowEvent::Warning {
                    message: format!(
                        "default device '{}' not found in current device list, ignoring",
                        default_device
                    ),
                })?;
                select_connected_device(devices, adapter)
            }
        };
    }

    select_connected_device(devices, adapter)
}

pub fn alias_device(
    config: &mut Config,
    devices: &[Device],
    device_identifier: &str,
    name: &str,
) -> Result<DeviceAliasResult> {
    let udid = resolve_device_id(device_identifier, config, devices)?;
    let device_name = devices
        .iter()
        .find(|d| d.identifier == udid)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let is_first = config.devices.aliases.is_empty();
    config
        .devices
        .aliases
        .insert(name.to_string(), udid.clone());
    if is_first {
        config.defaults.device = Some(name.to_string());
    }

    config.save()?;

    Ok(DeviceAliasResult {
        alias: name.to_string(),
        udid,
        device_name,
        is_default: is_first,
    })
}

fn select_connected_device(
    devices: &[Device],
    adapter: &mut impl WorkflowAdapter,
) -> Result<String> {
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
            let items: Vec<String> = connected
                .iter()
                .map(|d| format!("{} ({})", d.name, d.model))
                .collect();

            let selection = choose_index(
                adapter,
                "Multiple devices connected — choose one",
                &items,
                TossError::Device(
                    "multiple connected devices found — specify one with `--device`".into(),
                ),
            )?;

            Ok(connected[selection].identifier.clone())
        }
    }
}
