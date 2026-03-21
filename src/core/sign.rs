use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dialoguer::Select;
use tempfile::TempDir;

use super::error::{Result, TossError};
use super::project::extract_bundle_id;
use super::xcrun;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SigningIdentity {
    pub hash: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ProvisioningProfile {
    pub path: PathBuf,
    pub name: String,
    pub team_ids: Vec<String>,
    pub bundle_id_pattern: String,
}

pub struct ExtractedApp {
    pub _temp_dir: TempDir,
    pub app_path: PathBuf,
    pub bundle_id: String,
}

// ---------------------------------------------------------------------------
// IPA extraction
// ---------------------------------------------------------------------------

pub fn unzip_ipa(ipa_path: &Path) -> Result<ExtractedApp> {
    if !ipa_path.exists() {
        return Err(TossError::Signing(format!(
            "IPA not found: {}",
            ipa_path.display()
        )));
    }

    let temp_dir = TempDir::new()?;

    let output = Command::new("unzip")
        .args(["-q", "-o"])
        .arg(ipa_path)
        .arg("-d")
        .arg(temp_dir.path())
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "unzip failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    // Find Payload/*.app
    let payload_dir = temp_dir.path().join("Payload");
    if !payload_dir.is_dir() {
        return Err(TossError::Signing("IPA has no Payload/ directory".into()));
    }

    let app_path = fs::read_dir(&payload_dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "app"))
        .map(|e| e.path())
        .ok_or_else(|| TossError::Signing("no .app found in Payload/".into()))?;

    let bundle_id = extract_bundle_id(&app_path)?;

    Ok(ExtractedApp {
        _temp_dir: temp_dir,
        app_path,
        bundle_id,
    })
}

// ---------------------------------------------------------------------------
// Signing identity discovery
// ---------------------------------------------------------------------------

pub fn list_signing_identities() -> Result<Vec<SigningIdentity>> {
    let output = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "security find-identity failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut identities = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        // Lines look like:
        //   1) ABCDEF1234567890... "Apple Development: Name (TEAMID)"
        if let Some(rest) = trimmed.strip_prefix(|c: char| c.is_ascii_digit()) {
            let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == ')');
            let rest = rest.trim();
            // Extract hex hash (40 chars)
            if rest.len() >= 40 && rest[..40].chars().all(|c| c.is_ascii_hexdigit()) {
                let hash = rest[..40].to_string();
                // Extract quoted name
                if let (Some(start), Some(end)) = (rest.find('"'), rest.rfind('"'))
                    && start < end
                {
                    let name = rest[start + 1..end].to_string();
                    identities.push(SigningIdentity { hash, name });
                }
            }
        }
    }

    if identities.is_empty() {
        return Err(TossError::Signing(
            "no valid signing identities found in keychain".into(),
        ));
    }

    Ok(identities)
}

pub fn select_signing_identity(
    identities: &[SigningIdentity],
    override_name: Option<&str>,
) -> Result<SigningIdentity> {
    if let Some(query) = override_name {
        // Substring match
        let matches: Vec<_> = identities
            .iter()
            .filter(|id| id.name.contains(query) || id.hash.starts_with(query))
            .collect();
        return match matches.len() {
            0 => Err(TossError::Signing(format!(
                "no signing identity matching '{}'",
                query
            ))),
            1 => Ok(matches[0].clone()),
            _ => {
                let items: Vec<&str> = matches.iter().map(|id| id.name.as_str()).collect();
                let selection = Select::new()
                    .with_prompt("Multiple identities match, select one")
                    .items(&items)
                    .default(0)
                    .interact()
                    .map_err(|e| TossError::UserCancelled(e.to_string()))?;
                Ok(matches[selection].clone())
            }
        };
    }

    match identities.len() {
        1 => Ok(identities[0].clone()),
        _ => {
            let items: Vec<&str> = identities.iter().map(|id| id.name.as_str()).collect();
            let selection = Select::new()
                .with_prompt("Select signing identity")
                .items(&items)
                .default(0)
                .interact()
                .map_err(|e| TossError::UserCancelled(e.to_string()))?;
            Ok(identities[selection].clone())
        }
    }
}

// ---------------------------------------------------------------------------
// Provisioning profile discovery
// ---------------------------------------------------------------------------

pub fn find_provisioning_profiles() -> Result<Vec<ProvisioningProfile>> {
    let profiles_dir = dirs::home_dir()
        .ok_or_else(|| TossError::Signing("cannot determine home directory".into()))?
        .join("Library/MobileDevice/Provisioning Profiles");

    if !profiles_dir.is_dir() {
        return Err(TossError::Signing(format!(
            "provisioning profiles directory not found: {}",
            profiles_dir.display()
        )));
    }

    let mut profiles = Vec::new();

    for entry in fs::read_dir(&profiles_dir)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "mobileprovision")
            && let Ok(profile) = parse_provisioning_profile(&path)
        {
            profiles.push(profile);
        }
    }

    if profiles.is_empty() {
        return Err(TossError::Signing(
            "no provisioning profiles found in ~/Library/MobileDevice/Provisioning Profiles/"
                .into(),
        ));
    }

    Ok(profiles)
}

fn parse_provisioning_profile(path: &Path) -> Result<ProvisioningProfile> {
    // Decode the CMS envelope
    let cms_output = Command::new("security")
        .args(["cms", "-D", "-i"])
        .arg(path)
        .output()?;

    if !cms_output.status.success() {
        return Err(TossError::Signing(format!(
            "security cms failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&cms_output.stderr)
        )));
    }

    // Write decoded plist to a temp file, then convert to JSON with plutil
    let tmp = tempfile::NamedTempFile::new()?;
    fs::write(tmp.path(), &cms_output.stdout)?;

    let json_output = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(tmp.path())
        .output()?;

    if !json_output.status.success() {
        return Err(TossError::Signing(format!(
            "plutil convert failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&json_output.stderr)
        )));
    }

    let parsed: serde_json::Value = serde_json::from_slice(&json_output.stdout)?;

    let name = parsed["Name"].as_str().unwrap_or("Unknown").to_string();

    let team_ids: Vec<String> = parsed["TeamIdentifier"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // application-identifier looks like "TEAMID.com.example.app" or "TEAMID.*"
    let app_id = parsed["Entitlements"]["application-identifier"]
        .as_str()
        .unwrap_or("");
    let bundle_id_pattern = app_id
        .find('.')
        .map(|i| &app_id[i + 1..])
        .unwrap_or(app_id)
        .to_string();

    Ok(ProvisioningProfile {
        path: path.to_path_buf(),
        name,
        team_ids,
        bundle_id_pattern,
    })
}

pub fn match_profile(
    profiles: &[ProvisioningProfile],
    bundle_id: &str,
) -> Result<ProvisioningProfile> {
    // First pass: exact bundle ID match
    let exact: Vec<_> = profiles
        .iter()
        .filter(|p| p.bundle_id_pattern == bundle_id)
        .collect();

    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }

    // Second pass: wildcard match (e.g., "com.team.*" matches "com.team.app")
    let wildcard: Vec<_> = profiles
        .iter()
        .filter(|p| {
            if p.bundle_id_pattern == "*" {
                return true;
            }
            if let Some(prefix) = p.bundle_id_pattern.strip_suffix("*") {
                return bundle_id.starts_with(prefix);
            }
            p.bundle_id_pattern == bundle_id
        })
        .collect();

    let candidates = if !exact.is_empty() { &exact } else { &wildcard };

    match candidates.len() {
        0 => Err(TossError::Signing(format!(
            "no provisioning profile matches bundle ID '{}'",
            bundle_id
        ))),
        1 => Ok(candidates[0].clone()),
        _ => {
            let items: Vec<String> = candidates
                .iter()
                .map(|p| format!("{} ({})", p.name, p.bundle_id_pattern))
                .collect();
            let selection = Select::new()
                .with_prompt("Multiple profiles match, select one")
                .items(&items)
                .default(0)
                .interact()
                .map_err(|e| TossError::UserCancelled(e.to_string()))?;
            Ok(candidates[selection].clone())
        }
    }
}

fn load_profile_from_path(path: &Path) -> Result<ProvisioningProfile> {
    if !path.exists() {
        return Err(TossError::Signing(format!(
            "provisioning profile not found: {}",
            path.display()
        )));
    }
    parse_provisioning_profile(path)
}

// ---------------------------------------------------------------------------
// Entitlements extraction
// ---------------------------------------------------------------------------

fn extract_entitlements(profile_path: &Path, temp_dir: &Path) -> Result<PathBuf> {
    // Decode CMS
    let cms_output = Command::new("security")
        .args(["cms", "-D", "-i"])
        .arg(profile_path)
        .output()?;

    if !cms_output.status.success() {
        return Err(TossError::Signing(format!(
            "security cms failed: {}",
            String::from_utf8_lossy(&cms_output.stderr)
        )));
    }

    // Write decoded plist to temp file
    let decoded_plist = temp_dir.join("profile_decoded.plist");
    fs::write(&decoded_plist, &cms_output.stdout)?;

    // Extract Entitlements dict as xml1 plist
    let ent_path = temp_dir.join("entitlements.plist");
    let output = Command::new("plutil")
        .args([
            "-extract",
            "Entitlements",
            "xml1",
            "-o",
            &ent_path.to_string_lossy(),
            &decoded_plist.to_string_lossy(),
        ])
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "failed to extract entitlements: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(ent_path)
}

// ---------------------------------------------------------------------------
// Code signing
// ---------------------------------------------------------------------------

fn resign_app(app_path: &Path, identity: &SigningIdentity, entitlements: &Path) -> Result<()> {
    // Sign nested code first (frameworks, dylibs, plugins)
    sign_nested(app_path, identity, "Frameworks")?;
    sign_nested(app_path, identity, "PlugIns")?;

    // Sign the main app bundle with entitlements
    let output = Command::new("codesign")
        .args([
            "--force",
            "--sign",
            &identity.hash,
            "--entitlements",
            &entitlements.to_string_lossy(),
            &app_path.to_string_lossy(),
        ])
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "codesign failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}

fn sign_nested(app_path: &Path, identity: &SigningIdentity, subdir: &str) -> Result<()> {
    let dir = app_path.join(subdir);
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();

        let should_sign = ext == "framework" || ext == "dylib" || ext == "appex";

        if should_sign {
            // Recursively sign nested content inside frameworks
            if ext == "framework" || ext == "appex" {
                sign_nested(&path, identity, "Frameworks")?;
            }

            let output = Command::new("codesign")
                .args(["--force", "--sign", &identity.hash, &path.to_string_lossy()])
                .output()?;

            if !output.status.success() {
                return Err(TossError::Signing(format!(
                    "codesign failed for {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Top-level workflow
// ---------------------------------------------------------------------------

pub fn sign_workflow(
    ipa_path: &Path,
    device_id: &str,
    identity_override: Option<&str>,
    profile_override: Option<&str>,
    launch: bool,
) -> Result<()> {
    // 1. Extract IPA
    let extracted = unzip_ipa(ipa_path)?;
    println!(
        "Extracted: {} ({})",
        extracted.bundle_id,
        extracted.app_path.file_name().unwrap().to_string_lossy()
    );

    // 2. Resolve signing identity
    let identities = list_signing_identities()?;
    let identity = select_signing_identity(&identities, identity_override)?;
    println!("Identity: {}", identity.name);

    // 3. Resolve provisioning profile
    let profile = if let Some(path) = profile_override {
        load_profile_from_path(Path::new(path))?
    } else {
        let profiles = find_provisioning_profiles()?;
        match_profile(&profiles, &extracted.bundle_id)?
    };
    println!("Profile: {}", profile.name);

    // 4. Replace embedded.mobileprovision
    let embedded = extracted.app_path.join("embedded.mobileprovision");
    fs::copy(&profile.path, &embedded)?;

    // 5. Extract entitlements from profile
    let ent_path = extract_entitlements(&profile.path, extracted._temp_dir.path())?;

    // 6. Re-sign
    resign_app(&extracted.app_path, &identity, &ent_path)?;

    // 7. Install
    xcrun::install_app(device_id, &extracted.app_path)?;

    // 8. Optionally launch
    if launch {
        xcrun::launch_app(device_id, &extracted.bundle_id)?;
    }

    Ok(())
}
