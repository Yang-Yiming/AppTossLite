use std::collections::BTreeSet;
use std::path::Path;

use crate::cli::adapters::StrictCliAdapter;
use crate::core::config::Config;
use crate::core::error::Result;
use crate::core::{actions, project};
use crate::core::sign;
use crate::core::state;

pub fn identities() -> Result<()> {
    let identities = sign::list_signing_identities()?;

    println!("Signing identities ({})", identities.len());
    for identity in identities {
        println!("  {}", identity.name);
        println!("    hash: {}", identity.hash);
    }

    Ok(())
}

pub fn profiles(config: &Config) -> Result<()> {
    let inspections = sign::inspect_provisioning_profiles()?;
    let temp_prefix = config.signing.temp_bundle_prefix.as_deref();

    println!("Provisioning profiles ({})", inspections.len());
    if inspections.is_empty() {
        println!("  <none>");
        return Ok(());
    }

    for inspection in inspections {
        match inspection.profile {
            Some(profile) => {
                let is_temp = temp_prefix
                    .map(|value| profile.bundle_id_pattern.starts_with(value))
                    .unwrap_or(false);
                let marker = if is_temp { " [temp]" } else { "" };
                println!("  {}{}", profile.name, marker);
                println!("    bundle: {}", profile.bundle_id_pattern);
                println!(
                    "    team: {}",
                    if profile.team_ids.is_empty() {
                        "<unknown>".to_string()
                    } else {
                        profile.team_ids.join(", ")
                    }
                );
                if let Some(uuid) = profile.uuid {
                    println!("    uuid: {}", uuid);
                }
                println!(
                    "    devices: {}",
                    if profile.provisions_all_devices {
                        "all".to_string()
                    } else if profile.provisioned_devices.is_empty() {
                        "<unspecified>".to_string()
                    } else {
                        profile.provisioned_devices.len().to_string()
                    }
                );
                println!("    path: {}", profile.path.display());
            }
            None => {
                println!("  <parse failed>");
                println!("    path: {}", inspection.path.display());
                println!(
                    "    error: {}",
                    inspection.error.as_deref().unwrap_or("unknown error")
                );
            }
        }
    }

    Ok(())
}

pub fn teams(config: &Config) -> Result<()> {
    let mut config_teams = BTreeSet::new();
    if let Some(team_id) = &config.signing.team_id {
        config_teams.insert(team_id.clone());
    }

    let mut identity_teams = BTreeSet::new();
    if let Ok(identities) = sign::list_signing_identities() {
        for identity in identities {
            if let Some(team_id) = extract_team_id_from_identity_name(&identity.name) {
                identity_teams.insert(team_id);
            }
        }
    }

    let mut profile_teams = BTreeSet::new();
    if let Ok(inspections) = sign::inspect_provisioning_profiles() {
        for inspection in inspections {
            if let Some(profile) = inspection.profile {
                for team_id in profile.team_ids {
                    profile_teams.insert(team_id);
                }
            }
        }
    }

    println!("Teams");
    println!(
        "  config: {}",
        join_or_none(&config_teams)
    );
    println!(
        "  identities: {}",
        join_or_none(&identity_teams)
    );
    println!(
        "  profiles: {}",
        join_or_none(&profile_teams)
    );

    let dirs = state::provisioning_profile_dirs()?;
    if !dirs.is_empty() {
        println!("  profile_dirs: {}", display_paths(&dirs));
    }

    Ok(())
}

pub fn doctor(config: &Config, project_name: Option<&str>, device: Option<&str>) -> Result<()> {
    println!("Signing doctor");
    println!(
        "  config team: {}",
        config.signing.team_id.as_deref().unwrap_or("<unset>")
    );

    match sign::list_signing_identities() {
        Ok(identities) => {
            if identities.is_empty() {
                println!("  identities: <none>");
            } else {
                println!("  identities:");
                for identity in identities {
                    println!("    {}", identity.name);
                }
            }
        }
        Err(err) => println!("  identities: unavailable ({})", err),
    }

    match sign::inspect_provisioning_profiles() {
        Ok(inspections) => {
            let profile_count = inspections.iter().filter(|i| i.profile.is_some()).count();
            let teams: BTreeSet<String> = inspections
                .into_iter()
                .filter_map(|inspection| inspection.profile)
                .flat_map(|profile| profile.team_ids)
                .collect();
            println!("  parsed profiles: {}", profile_count);
            println!("  profile teams: {}", join_or_none(&teams));
        }
        Err(err) => println!("  profiles: unavailable ({})", err),
    }

    if let Some(project_name) = resolve_ipa_project_name(config, project_name)? {
        let mut adapter = StrictCliAdapter;
        let (_device_id, device_udid, device_name) = actions::resolve_device(device, config, &mut adapter)?;
        let ipa_path = project::managed_ipa_path(config, &project_name)?;
        println!("  project: {}", project_name);
        println!("  device: {} ({})", device_name, device_udid);
        match sign::preview_signing_plan(config, &ipa_path, &device_udid, None, None) {
            Ok(preview) => render_preview(&preview, true),
            Err(err) => println!("  signing preview: failed ({})", err),
        }
    }

    Ok(())
}

pub fn preview_ipa_install(
    config: &Config,
    ipa_path: &Path,
    device: Option<&str>,
    identity: Option<&str>,
    profile: Option<&str>,
) -> Result<()> {
    let mut adapter = StrictCliAdapter;
    let (_device_id, device_udid, device_name) = actions::resolve_device(device, config, &mut adapter)?;
    let preview = sign::preview_signing_plan(config, ipa_path, &device_udid, identity, profile)?;
    println!("Dry run");
    println!("  device: {} ({})", device_name, device_udid);
    render_preview(&preview, false);
    Ok(())
}

pub fn preview_project_install(
    config: &Config,
    project_name: &str,
    device: Option<&str>,
    prebuilt: bool,
    run_after_install: bool,
) -> Result<()> {
    let mut adapter = StrictCliAdapter;
    let (_device_id, device_udid, device_name) = actions::resolve_device(device, config, &mut adapter)?;
    let project = config.projects.get(project_name).expect("validated project exists");

    println!("Dry run");
    println!("  project: {}", project_name);
    println!("  device: {} ({})", device_name, device_udid);
    println!("  mode: {}", if run_after_install { "install + launch" } else { "install only" });

    if project.is_ipa() {
        let ipa_path = project::managed_ipa_path(config, project_name)?;
        let preview = sign::preview_signing_plan(config, &ipa_path, &device_udid, None, None)?;
        render_preview(&preview, false);
        return Ok(());
    }

    println!("  project type: xcode/app");
    println!("  prebuilt: {}", prebuilt);
    if prebuilt {
        let (app_path, bundle_id) = project::resolve_project(config, project_name)?;
        println!("  app path: {}", app_path.display());
        println!("  bundle id: {}", bundle_id);
    } else {
        println!("  next step: build with xcodebuild for device {}", device_udid);
        if let Some(path) = &project.path {
            println!("  source path: {}", path);
        }
    }

    Ok(())
}

fn extract_team_id_from_identity_name(name: &str) -> Option<String> {
    let start = name.rfind('(')?;
    let end = name.rfind(')')?;
    if end <= start + 1 {
        return None;
    }

    let candidate = &name[start + 1..end];
    if candidate.len() == 10 && candidate.chars().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()) {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn join_or_none(values: &BTreeSet<String>) -> String {
    if values.is_empty() {
        "<none>".to_string()
    } else {
        values.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

fn display_paths(paths: &[std::path::PathBuf]) -> String {
    paths.iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolve_ipa_project_name(config: &Config, project_name: Option<&str>) -> Result<Option<String>> {
    if let Some(name) = project_name {
        let project = config.projects.get(name).ok_or_else(|| {
            crate::core::error::TossError::Project(format!(
                "unknown project '{}' — register it with `toss projects add`",
                name
            ))
        })?;
        if !project.is_ipa() {
            return Ok(None);
        }
        return Ok(Some(name.to_string()));
    }

    if let Some(default_project) = &config.defaults.project
        && config
            .projects
            .get(default_project)
            .is_some_and(|project| project.is_ipa())
    {
        return Ok(Some(default_project.clone()));
    }

    Ok(None)
}

fn render_preview(preview: &sign::SigningPreview, compact: bool) {
    if compact {
        println!("  app: {} ({})", preview.app_name, preview.extracted_bundle_id);
        println!("  identity: {}", preview.selected_identity_name);
    } else {
        println!("  app: {} ({})", preview.app_name, preview.extracted_bundle_id);
        println!("  identity: {}", preview.selected_identity_name);
        println!(
            "  config team: {}",
            preview.config_team_id.as_deref().unwrap_or("<unset>")
        );
    }

    for target in &preview.targets {
        println!(
            "  target: {} {} -> {}",
            target.kind, target.original_bundle_id, target.final_bundle_id
        );
        match &target.selected_profile_name {
            Some(profile_name) => {
                let teams = if target.selected_profile_team_ids.is_empty() {
                    "<unknown>".to_string()
                } else {
                    target.selected_profile_team_ids.join(", ")
                };
                println!("    profile: {}", profile_name);
                println!("    team: {}", teams);
            }
            None if target.requires_auto_provisioning => {
                println!("    profile: <missing>");
                println!("    next: auto-provisioning required");
            }
            None => println!("    profile: <none>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_team_id_from_identity_name;

    #[test]
    fn extracts_team_id_from_identity_name() {
        assert_eq!(
            extract_team_id_from_identity_name("Apple Development: test@example.com (VLXZVT5H87)"),
            Some("VLXZVT5H87".into())
        );
    }

    #[test]
    fn rejects_identity_name_without_team_suffix() {
        assert_eq!(
            extract_team_id_from_identity_name("Apple Development: test@example.com"),
            None
        );
    }
}
