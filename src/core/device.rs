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
    Paired,
    Disconnected,
    Unknown(String),
}

impl std::fmt::Display for DeviceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceState::Connected => write!(f, "connected"),
            DeviceState::Paired => write!(f, "paired"),
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
    if let Some(stored_id) = config.devices.aliases.get(identifier) {
        return find_device(stored_id, devices)
            .map(|device| device.identifier.clone())
            .ok_or_else(|| {
                TossError::Device(format!(
                    "device alias '{}' points to '{}' but that device is not currently available",
                    identifier, stored_id
                ))
            });
    }

    // Check if it's a currently available identifier or UDID directly
    if let Some(device) = find_device(identifier, devices) {
        return Ok(device.identifier.clone());
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
    let identifier = resolve_device_id(device_identifier, config, devices)?;
    let device = resolve_alias_target(&identifier, devices, device_identifier)?;

    let is_first = config.devices.aliases.is_empty();
    config
        .devices
        .aliases
        .insert(name.to_string(), device.udid.clone());
    if is_first {
        config.defaults.device = Some(name.to_string());
    }

    config.save()?;

    Ok(DeviceAliasResult {
        alias: name.to_string(),
        udid: device.udid.clone(),
        device_name: device.name.clone(),
        is_default: is_first,
    })
}

fn find_device<'a>(identifier: &str, devices: &'a [Device]) -> Option<&'a Device> {
    devices
        .iter()
        .find(|d| d.identifier == identifier || d.udid == identifier)
}

fn resolve_alias_target<'a>(
    identifier: &str,
    devices: &'a [Device],
    original_input: &str,
) -> Result<&'a Device> {
    find_device(identifier, devices).ok_or_else(|| {
        TossError::Device(format!(
            "device '{}' is no longer available for aliasing",
            original_input
        ))
    })
}

fn select_connected_device(
    devices: &[Device],
    adapter: &mut impl WorkflowAdapter,
) -> Result<String> {
    let connected: Vec<&Device> = devices
        .iter()
        .filter(|d| matches!(d.state, DeviceState::Connected | DeviceState::Paired))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{Config, DevicesConfig};
    use crate::core::interaction::{WorkflowAdapter, WorkflowEvent};

    struct TestAdapter;

    impl WorkflowAdapter for TestAdapter {
        fn emit(&mut self, _event: WorkflowEvent) -> Result<()> {
            Ok(())
        }

        fn choose(
            &mut self,
            _prompt: &str,
            _items: &[String],
            _default: usize,
        ) -> Result<Option<usize>> {
            Ok(None)
        }
    }

    fn device(identifier: &str, udid: &str) -> Device {
        Device {
            name: "Phone".into(),
            identifier: identifier.into(),
            udid: udid.into(),
            model: "iPhone".into(),
            os_version: "18.0".into(),
            state: DeviceState::Connected,
        }
    }

    #[test]
    fn resolves_alias_to_current_identifier_via_udid() {
        let config = Config {
            devices: DevicesConfig {
                aliases: [("phone".into(), "real-udid".into())].into_iter().collect(),
            },
            ..Config::default()
        };
        let devices = vec![device("devicectl-id", "real-udid")];

        let resolved = resolve_device_id("phone", &config, &devices).unwrap();

        assert_eq!(resolved, "devicectl-id");
    }

    #[test]
    fn rejects_stale_alias_when_device_is_missing() {
        let config = Config {
            devices: DevicesConfig {
                aliases: [("phone".into(), "missing-udid".into())]
                    .into_iter()
                    .collect(),
            },
            ..Config::default()
        };

        let err = resolve_device_id("phone", &config, &[]).unwrap_err();

        assert!(err.to_string().contains("not currently available"));
    }

    #[test]
    fn resolves_alias_target_to_udid_even_when_selected_by_index() {
        let devices = vec![device("devicectl-id", "real-udid")];
        let identifier = "devicectl-id";

        let aliased = resolve_alias_target(identifier, &devices, "1").unwrap();

        assert_eq!(aliased.udid, "real-udid");
    }

    #[test]
    fn falls_back_when_default_device_alias_is_stale() {
        let config = Config {
            defaults: crate::core::config::DefaultsConfig {
                device: Some("phone".into()),
                project: None,
            },
            devices: DevicesConfig {
                aliases: [("phone".into(), "missing-udid".into())]
                    .into_iter()
                    .collect(),
            },
            ..Config::default()
        };
        let devices = vec![device("devicectl-id", "real-udid")];

        let resolved = select_device(None, &config, &devices, &mut TestAdapter).unwrap();

        assert_eq!(resolved, "devicectl-id");
    }

    #[test]
    fn paired_devices_are_still_selectable() {
        let config = Config::default();
        let devices = vec![Device {
            name: "Phone".into(),
            identifier: "devicectl-id".into(),
            udid: "real-udid".into(),
            model: "iPhone".into(),
            os_version: "18.0".into(),
            state: DeviceState::Paired,
        }];

        let resolved = select_device(None, &config, &devices, &mut TestAdapter).unwrap();

        assert_eq!(resolved, "devicectl-id");
    }
}
