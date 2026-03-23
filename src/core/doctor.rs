use std::process::Command;

use crate::core::config::Config;
use crate::core::device::DeviceState;
use crate::core::error::Result;
use crate::core::sign;
use crate::core::state;
use crate::core::xcrun;

#[derive(Debug, Clone)]
pub struct DoctorLine {
    pub status: &'static str,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct DoctorSection {
    pub title: &'static str,
    pub lines: Vec<DoctorLine>,
}

#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub sections: Vec<DoctorSection>,
    pub failures: usize,
    pub warnings: usize,
}

pub fn collect(config: &Config) -> Result<DoctorReport> {
    let mut report = DoctorAccumulator::default();
    let config_path = Config::path()?;
    let config_exists = config_path.exists();

    report.line(
        "Config",
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
        "Config",
        "INFO",
        "stored here",
        "defaults, device aliases, projects, temp_bundle_prefix, team_id".to_string(),
    );
    report.line(
        "Config",
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
        "Config",
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
        "Config",
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
        "Config",
        "INFO",
        "default_project",
        config
            .defaults
            .project
            .as_deref()
            .unwrap_or("<unset>")
            .to_string(),
    );

    match run_command(["xcode-select", "-p"]) {
        Ok(stdout) => report.line("Xcode", "PASS", "xcode-select", stdout.trim().to_string()),
        Err(err) => report.line("Xcode", "FAIL", "xcode-select", err),
    }
    match run_command(["xcodebuild", "-version"]) {
        Ok(stdout) => report.line("Xcode", "PASS", "xcodebuild", stdout.trim().to_string()),
        Err(err) => report.line("Xcode", "FAIL", "xcodebuild", err),
    }

    match sign::list_signing_identities() {
        Ok(identities) => {
            report.line(
                "Signing Identities",
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
                        "Signing Identities",
                        "FAIL",
                        "team match",
                        format!("no signing identity matches team_id '{}'", team_id),
                    );
                } else {
                    report.line(
                        "Signing Identities",
                        "PASS",
                        "team match",
                        format!("{} identity(ies) match '{}'", matches.len(), team_id),
                    );
                }
            }
        }
        Err(err) => report.line(
            "Signing Identities",
            "FAIL",
            "codesigning identities",
            err.to_string(),
        ),
    }

    let profile_dirs = state::provisioning_profile_dirs()?;
    if profile_dirs.is_empty() {
        report.line(
            "Provisioning Cache",
            "WARN",
            "profile dirs",
            "<none>".to_string(),
        );
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
                "Provisioning Cache",
                "INFO",
                "profile dir",
                format!("{} ({} files)", dir.display(), file_count),
            );
        }
        report.line(
            "Provisioning Cache",
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
                "Provisioning Cache",
                if parsed.is_empty() { "WARN" } else { "PASS" },
                "parsed profiles",
                format!("{} parsed, {} failed", parsed.len(), failed.len()),
            );

            for item in failed {
                report.line(
                    "Provisioning Cache",
                    "WARN",
                    "parse failed",
                    format!(
                        "{} -> {}",
                        item.path.display(),
                        item.error.as_deref().unwrap_or("unknown error")
                    ),
                );
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
                    "Provisioning Cache",
                    if temp_profiles > 0 { "PASS" } else { "WARN" },
                    "temp profiles",
                    format!("{} matching prefix '{}'", temp_profiles, prefix),
                );
            }
        }
        Err(err) => report.line(
            "Provisioning Cache",
            "WARN",
            "parsed profiles",
            err.to_string(),
        ),
    }

    match xcrun::list_devices() {
        Ok(devices) => {
            let connected = devices
                .iter()
                .filter(|d| matches!(d.state, DeviceState::Connected))
                .count();
            report.line(
                "Devices",
                "PASS",
                "connected devices",
                format!("{} of {} detected", connected, devices.len()),
            );
        }
        Err(err) => report.line("Devices", "WARN", "connected devices", err.to_string()),
    }

    Ok(report.finish())
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

#[derive(Default)]
struct DoctorAccumulator {
    sections: Vec<DoctorSection>,
    failures: usize,
    warnings: usize,
}

impl DoctorAccumulator {
    fn line(&mut self, title: &'static str, status: &'static str, label: &str, detail: String) {
        match status {
            "FAIL" => self.failures += 1,
            "WARN" => self.warnings += 1,
            _ => {}
        }

        if let Some(section) = self
            .sections
            .iter_mut()
            .find(|section| section.title == title)
        {
            section.lines.push(DoctorLine {
                status,
                label: label.to_string(),
                detail,
            });
        } else {
            self.sections.push(DoctorSection {
                title,
                lines: vec![DoctorLine {
                    status,
                    label: label.to_string(),
                    detail,
                }],
            });
        }
    }

    fn finish(self) -> DoctorReport {
        DoctorReport {
            sections: self.sections,
            failures: self.failures,
            warnings: self.warnings,
        }
    }
}
