use std::path::PathBuf;
use std::process::Command;

use crate::core::config::Config;
use crate::core::error::{Result, TossError};
use crate::core::sign;
use crate::core::xcrun;

pub fn run(config: &Config) -> Result<()> {
    let mut report = DoctorReport::new();
    let config_path = Config::path()?;
    let config_exists = config_path.exists();

    println!("toss doctor");
    println!();

    println!("Config");
    report.line(
        if config_exists { "PASS" } else { "WARN" },
        "config file",
        format!(
            "{}{}",
            config_path.display(),
            if config_exists {
                ""
            } else {
                " (not created yet)"
            }
        ),
    );
    report.line(
        "INFO",
        "stored here",
        "defaults, device aliases, projects, temp_bundle_prefix, team_id".to_string(),
    );
    report.line(
        if config.signing.team_id.is_some() {
            "PASS"
        } else {
            "FAIL"
        },
        "team_id",
        config
            .signing
            .team_id
            .as_deref()
            .unwrap_or("<unset>")
            .to_string(),
    );
    report.line(
        if config.signing.temp_bundle_prefix.is_some() {
            "PASS"
        } else {
            "WARN"
        },
        "temp_bundle_prefix",
        config
            .signing
            .temp_bundle_prefix
            .as_deref()
            .unwrap_or("<unset>")
            .to_string(),
    );
    report.line(
        "INFO",
        "default_device",
        config
            .defaults
            .device
            .as_deref()
            .unwrap_or("<unset>")
            .to_string(),
    );
    report.line(
        "INFO",
        "default_project",
        config
            .defaults
            .project
            .as_deref()
            .unwrap_or("<unset>")
            .to_string(),
    );
    println!();

    println!("Xcode");
    match run_command(["xcode-select", "-p"]) {
        Ok(stdout) => report.line("PASS", "xcode-select", stdout.trim().to_string()),
        Err(err) => report.line("FAIL", "xcode-select", err),
    }
    match run_command(["xcodebuild", "-version"]) {
        Ok(stdout) => report.line("PASS", "xcodebuild", stdout.trim().to_string()),
        Err(err) => report.line("FAIL", "xcodebuild", err),
    }
    println!();

    println!("Signing Identities");
    match sign::list_signing_identities() {
        Ok(identities) => {
            report.line(
                "PASS",
                "codesigning identities",
                format!("{} found", identities.len()),
            );

            if let Some(team_id) = config.signing.team_id.as_deref() {
                let matches: Vec<_> = identities
                    .iter()
                    .filter(|id| id.name.contains(&format!("({})", team_id)))
                    .collect();

                if matches.is_empty() {
                    report.line(
                        "FAIL",
                        "team match",
                        format!("no signing identity matches team_id '{}'", team_id),
                    );
                } else {
                    report.line(
                        "PASS",
                        "team match",
                        format!("{} identity(ies) match '{}'", matches.len(), team_id),
                    );
                }
            }
        }
        Err(err) => report.line("FAIL", "codesigning identities", err.to_string()),
    }
    println!();

    println!("Provisioning Cache");
    let profile_dirs = provisioning_profile_dirs()?;
    if profile_dirs.is_empty() {
        report.line("WARN", "profile dirs", "<none>".to_string());
    } else {
        for dir in &profile_dirs {
            let file_count = std::fs::read_dir(dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "mobileprovision")
                })
                .count();
            report.line(
                "INFO",
                "profile dir",
                format!("{} ({} files)", dir.display(), file_count),
            );
        }
        report.line(
            "INFO",
            "stored here",
            "downloaded Xcode provisioning profiles".to_string(),
        );
    }

    match sign::inspect_provisioning_profiles() {
        Ok(inspections) => {
            let parsed: Vec<_> = inspections.iter().filter(|i| i.profile.is_some()).collect();
            let failed: Vec<_> = inspections.iter().filter(|i| i.profile.is_none()).collect();

            report.line(
                if parsed.is_empty() { "WARN" } else { "PASS" },
                "parsed profiles",
                format!("{} parsed, {} failed", parsed.len(), failed.len()),
            );

            if !failed.is_empty() {
                for item in failed {
                    report.line(
                        "WARN",
                        "parse failed",
                        format!(
                            "{} -> {}",
                            item.path.display(),
                            item.error.as_deref().unwrap_or("unknown error")
                        ),
                    );
                }
            }

            if let Some(prefix) = config.signing.temp_bundle_prefix.as_deref() {
                let temp_profiles = parsed
                    .iter()
                    .filter(|inspection| {
                        inspection
                            .profile
                            .as_ref()
                            .is_some_and(|profile| profile.bundle_id_pattern.starts_with(prefix))
                    })
                    .count();
                report.line(
                    if temp_profiles > 0 { "PASS" } else { "WARN" },
                    "temp profiles",
                    format!("{} matching prefix '{}'", temp_profiles, prefix),
                );
            }
        }
        Err(err) => report.line("WARN", "parsed profiles", err.to_string()),
    }
    println!();

    println!("Devices");
    match xcrun::list_devices() {
        Ok(devices) => {
            let connected = devices
                .iter()
                .filter(|d| matches!(d.state, crate::core::device::DeviceState::Connected))
                .count();
            report.line(
                "PASS",
                "connected devices",
                format!("{} of {} detected", connected, devices.len()),
            );
        }
        Err(err) => report.line("WARN", "connected devices", err.to_string()),
    }
    println!();

    report.finish()
}

fn run_command<const N: usize>(args: [&str; N]) -> std::result::Result<String, String> {
    let mut command = Command::new(args[0]);
    command.args(&args[1..]);
    let output = command.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn provisioning_profile_dirs() -> Result<Vec<PathBuf>> {
    let home = dirs::home_dir()
        .ok_or_else(|| TossError::Config("cannot determine home directory".into()))?;

    Ok([
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ]
    .into_iter()
    .filter(|path| path.is_dir())
    .collect())
}

struct DoctorReport {
    failures: usize,
    warnings: usize,
}

impl DoctorReport {
    fn new() -> Self {
        Self {
            failures: 0,
            warnings: 0,
        }
    }

    fn line(&mut self, status: &str, label: &str, detail: String) {
        match status {
            "FAIL" => self.failures += 1,
            "WARN" => self.warnings += 1,
            _ => {}
        }

        println!("  [{:<4}] {:<20} {}", status, label, detail);
    }

    fn finish(self) -> Result<()> {
        println!(
            "Summary: {} failure(s), {} warning(s)",
            self.failures, self.warnings
        );

        if self.failures > 0 {
            Err(TossError::Config(format!(
                "doctor found {} failure(s) and {} warning(s)",
                self.failures, self.warnings
            )))
        } else {
            Ok(())
        }
    }
}
