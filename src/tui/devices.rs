use console::Style;
use dialoguer::{Input, Select};

use crate::core::config::Config;
use crate::core::device::{self, DeviceState};
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
    let devices = xcrun::list_devices()?;

    if devices.is_empty() {
        println!("No devices found.");
        return Ok(());
    }

    let green = Style::new().green();
    let dim = Style::new().dim();

    let alias_map: std::collections::HashMap<&str, &str> = config
        .devices
        .aliases
        .iter()
        .map(|(name, udid)| (udid.as_str(), name.as_str()))
        .collect();

    let mut rows: Vec<(String, String, String, String, String)> = Vec::new();
    for (i, d) in devices.iter().enumerate() {
        let idx = format!("{}", i + 1);
        let alias = alias_map
            .get(d.identifier.as_str())
            .map(|a| format!(" ({})", a))
            .unwrap_or_default();
        rows.push((
            idx,
            format!("{}{}", d.name, alias),
            d.model.clone(),
            d.os_version.clone(),
            d.state.to_string(),
        ));
    }

    let w_idx = rows.iter().map(|r| r.0.len()).max().unwrap_or(1);
    let w_name = rows.iter().map(|r| r.1.len()).max().unwrap_or(4).max(4);
    let w_model = rows.iter().map(|r| r.2.len()).max().unwrap_or(5).max(5);
    let w_os = rows.iter().map(|r| r.3.len()).max().unwrap_or(2).max(2);

    println!(
        "  {:<w_idx$}  {:<w_name$}  {:<w_model$}  {:<w_os$}  {}",
        "#", "Name", "Model", "OS", "State",
    );
    println!(
        "  {:<w_idx$}  {:<w_name$}  {:<w_model$}  {:<w_os$}  {}",
        "-".repeat(w_idx),
        "-".repeat(w_name),
        "-".repeat(w_model),
        "-".repeat(w_os),
        "-----",
    );

    for (i, (idx, name, model, os, state)) in rows.iter().enumerate() {
        let styled_state = if devices[i].state == DeviceState::Connected {
            green.apply_to(state).to_string()
        } else {
            dim.apply_to(state).to_string()
        };
        let styled_name = if devices[i].state == DeviceState::Connected {
            name.clone()
        } else {
            dim.apply_to(name).to_string()
        };
        println!(
            "  {:<w_idx$}  {:<w_name$}  {:<w_model$}  {:<w_os$}  {}",
            idx, styled_name, model, os, styled_state,
        );
    }

    Ok(())
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

    let device_id = &connected[selection].identifier;

    let name: String = Input::new()
        .with_prompt("Alias name")
        .interact_text()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    let aliased = device::alias_device(config, &devices, device_id, &name)?;

    println!(
        "Aliased '{}' → {} ({})",
        aliased.alias, aliased.device_name, aliased.udid
    );
    if aliased.is_default {
        let dim = console::Style::new().dim();
        println!("{}", dim.apply_to("  (set as default device)"));
    }
    Ok(())
}
