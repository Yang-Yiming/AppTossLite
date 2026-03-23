use console::Style;

use crate::core::config::Config;
use crate::core::device::{self, DeviceState};
use crate::core::error::Result;
use crate::core::xcrun;

pub fn list(config: &Config) -> Result<()> {
    let devices = xcrun::list_devices()?;

    if devices.is_empty() {
        println!("No devices found.");
        return Ok(());
    }

    let green = Style::new().green();
    let dim = Style::new().dim();

    // Build reverse alias map: UDID -> alias name
    let alias_map: std::collections::HashMap<&str, &str> = config
        .devices
        .aliases
        .iter()
        .map(|(name, udid)| (udid.as_str(), name.as_str()))
        .collect();

    // Calculate column widths
    let mut rows: Vec<(String, String, String, String, String)> = Vec::new();
    for (i, d) in devices.iter().enumerate() {
        let idx = format!("{}", i + 1);
        let alias = alias_map
            .get(d.identifier.as_str())
            .map(|a| format!(" ({})", a))
            .unwrap_or_default();
        let name = format!("{}{}", d.name, alias);
        let model = d.model.clone();
        let os = d.os_version.clone();
        let state = d.state.to_string();
        rows.push((idx, name, model, os, state));
    }

    let w_idx = rows.iter().map(|r| r.0.len()).max().unwrap_or(1);
    let w_name = rows.iter().map(|r| r.1.len()).max().unwrap_or(4).max(4);
    let w_model = rows.iter().map(|r| r.2.len()).max().unwrap_or(5).max(5);
    let w_os = rows.iter().map(|r| r.3.len()).max().unwrap_or(2).max(2);

    // Header
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

pub fn alias(config: &mut Config, device: &str, name: &str) -> Result<()> {
    let devices = xcrun::list_devices()?;
    let aliased = device::alias_device(config, &devices, device, name)?;

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
