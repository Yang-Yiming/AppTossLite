use console::Style;

use crate::core::config::Config;
use crate::core::device::select_device;
use crate::core::error::Result;
use crate::core::sign;
use crate::core::xcrun;

pub fn sign(
    config: &Config,
    ipa: &str,
    device: Option<&str>,
    identity: Option<&str>,
    profile: Option<&str>,
    launch: bool,
) -> Result<()> {
    let ipa_path = std::path::Path::new(ipa);

    let devices = xcrun::list_devices()?;
    let device_id = select_device(device, config, &devices)?;
    let device_name = devices
        .iter()
        .find(|d| d.identifier == device_id)
        .map(|d| d.name.as_str())
        .unwrap_or(&device_id);

    let bold = Style::new().bold();
    let ipa_name = ipa_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ipa.to_string());
    println!(
        "Signing {} → {}...",
        bold.apply_to(&ipa_name),
        bold.apply_to(device_name),
    );

    sign::sign_workflow(ipa_path, &device_id, identity, profile, launch)?;

    let green = Style::new().green().bold();
    if launch {
        println!("{}", green.apply_to("Running!"));
    } else {
        println!("{}", green.apply_to("Installed successfully."));
    }
    Ok(())
}
