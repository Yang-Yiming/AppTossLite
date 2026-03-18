use console::Style;
use dialoguer::{Input, Select};

use crate::core::config::Config;
use crate::core::device::{DeviceState, resolve_device_id};
use crate::core::error::{Result, TossError};
use crate::core::xcrun;

pub fn menu(config: &mut Config) -> Result<()> {
    loop {
        let items = &["List devices", "Alias a device", "Back"];

        let selection = Select::new()
            .with_prompt("Devices")
            .items(items)
            .default(0)
            .interact()
            .map_err(|e| TossError::UserCancelled(e.to_string()))?;

        match selection {
            0 => list(config)?,
            1 => {
                if let Err(e) = alias(config) {
                    let red = Style::new().red().bold();
                    eprintln!("{} {}", red.apply_to("error:"), e);
                }
            }
            2 => return Ok(()),
            _ => unreachable!(),
        }
    }
}

fn list(config: &Config) -> Result<()> {
    crate::cli::devices::list(config)
}

fn alias(config: &mut Config) -> Result<()> {
    let devices = xcrun::list_devices()?;
    let connected: Vec<_> = devices
        .iter()
        .filter(|d| d.state == DeviceState::Connected)
        .collect();

    if connected.is_empty() {
        println!("No connected devices found.");
        return Ok(());
    }

    let items: Vec<String> = connected
        .iter()
        .map(|d| format!("{} ({})", d.name, d.model))
        .collect();

    let selection = Select::new()
        .with_prompt("Select device to alias")
        .items(&items)
        .default(0)
        .interact()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    let udid = &connected[selection].identifier;

    let name: String = Input::new()
        .with_prompt("Alias name")
        .interact_text()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    // Verify the device resolves correctly
    resolve_device_id(udid, config, &devices)?;

    config.devices.aliases.insert(name.clone(), udid.clone());
    config.save()?;

    let device_name = &connected[selection].name;
    println!("Aliased '{}' → {} ({})", name, device_name, udid);
    Ok(())
}
