use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dialoguer::Select;
use tempfile::TempDir;

use super::config::Config;
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

#[derive(Debug, Clone)]
pub struct ProvisioningProfileInspection {
    pub path: PathBuf,
    pub profile: Option<ProvisioningProfile>,
    pub error: Option<String>,
}

pub struct ExtractedApp {
    pub _temp_dir: TempDir,
    pub app_path: PathBuf,
    pub bundle_id: String,
}

struct SigningPlan {
    profile: ProvisioningProfile,
    final_bundle_id: String,
    cleanup_profiles: Vec<PathBuf>,
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

fn ensure_no_app_extensions(app_path: &Path) -> Result<()> {
    let plugins_dir = app_path.join("PlugIns");
    if !plugins_dir.is_dir() {
        return Ok(());
    }

    let has_appex = fs::read_dir(&plugins_dir)?
        .filter_map(|e| e.ok())
        .any(|entry| entry.path().extension().is_some_and(|ext| ext == "appex"));

    if has_appex {
        return Err(TossError::Signing(
            "temporary bundle ID signing does not support apps with .appex extensions yet".into(),
        ));
    }

    Ok(())
}

fn plist_set_string(plist: &Path, key: &str, value: &str) -> Result<()> {
    let output = Command::new("plutil")
        .args(["-replace", key, "-string", value])
        .arg(plist)
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "failed to update {} in {}: {}",
            key,
            plist.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

fn rewrite_bundle_id(app_path: &Path, bundle_id: &str) -> Result<()> {
    let info_plist = app_path.join("Info.plist");
    plist_set_string(&info_plist, "CFBundleIdentifier", bundle_id)
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
    let profiles_dir = provisioning_profiles_dir()?;

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
        return Err(TossError::Signing(format!(
            "no provisioning profiles found in {} — download them via Xcode → Settings → Accounts",
            profiles_dir.display()
        )));
    }

    Ok(profiles)
}

pub fn inspect_provisioning_profiles() -> Result<Vec<ProvisioningProfileInspection>> {
    let profiles_dir = provisioning_profiles_dir()?;
    let mut inspections = Vec::new();

    for entry in fs::read_dir(&profiles_dir)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "mobileprovision") {
            continue;
        }

        match parse_provisioning_profile(&path) {
            Ok(profile) => inspections.push(ProvisioningProfileInspection {
                path,
                profile: Some(profile),
                error: None,
            }),
            Err(err) => inspections.push(ProvisioningProfileInspection {
                path,
                profile: None,
                error: Some(err.to_string()),
            }),
        }
    }

    inspections.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(inspections)
}

fn provisioning_profiles_dir() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| TossError::Signing("cannot determine home directory".into()))?;

    let candidates = [
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ];

    candidates
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .ok_or_else(|| {
            TossError::Signing(
                "no provisioning profiles directory found — open Xcode → Settings → Accounts → Download Manual Profiles".into(),
            )
        })
}

fn plutil_extract(plist: &Path, key: &str) -> Option<String> {
    let output = Command::new("plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
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

    // Write decoded plist to temp file, then extract fields individually.
    // (plutil -convert json fails on profiles because they contain <data> blobs)
    let tmp = tempfile::NamedTempFile::new()?;
    fs::write(tmp.path(), &cms_output.stdout)?;

    let name = plutil_extract(tmp.path(), "Name").unwrap_or_else(|| "Unknown".into());

    // Collect TeamIdentifier array
    let mut team_ids = Vec::new();
    for i in 0..8 {
        match plutil_extract(tmp.path(), &format!("TeamIdentifier.{}", i)) {
            Some(id) => team_ids.push(id),
            None => break,
        }
    }

    // application-identifier looks like "TEAMID.com.example.app" or "TEAMID.*"
    let app_id =
        plutil_extract(tmp.path(), "Entitlements.application-identifier").unwrap_or_default();
    let bundle_id_pattern = app_id
        .find('.')
        .map(|i| app_id[i + 1..].to_string())
        .unwrap_or(app_id);

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

fn list_profile_paths(profiles: &[ProvisioningProfile]) -> Vec<PathBuf> {
    profiles.iter().map(|p| p.path.clone()).collect()
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

fn cleanup_profiles(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-provisioning
// ---------------------------------------------------------------------------

fn extract_team_id(identity: &SigningIdentity) -> Option<String> {
    // Identity name looks like "Apple Development: Name (TEAMID)"
    let start = identity.name.rfind('(')?;
    let end = identity.name.rfind(')')?;
    (start < end).then(|| identity.name[start + 1..end].to_string())
}

fn normalize_bundle_component(input: &str) -> String {
    let mut out = String::new();
    let mut prev_was_sep = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_was_sep = false;
        } else if !prev_was_sep {
            out.push('-');
            prev_was_sep = true;
        }
    }

    out.trim_matches('-').to_string()
}

fn stable_hex_suffix(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (hash & 0xffff_ffff) as u32)
}

fn validate_bundle_prefix(prefix: &str) -> bool {
    prefix
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

fn temp_bundle_id_prefix(config: &Config) -> Result<&str> {
    let prefix = config
        .signing
        .temp_bundle_prefix
        .as_deref()
        .ok_or_else(|| {
            TossError::Signing(
                "temporary bundle ID fallback requires `toss config set-temp-bundle-prefix <prefix>`"
                    .into(),
            )
        })?;

    if !validate_bundle_prefix(prefix) {
        return Err(TossError::Signing(format!(
            "invalid configured temp bundle prefix '{}' — update it with `toss config set-temp-bundle-prefix <prefix>`",
            prefix
        )));
    }

    Ok(prefix)
}

fn display_app_name(app_path: &Path, original_bundle_id: &str) -> String {
    let from_path = app_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let normalized = normalize_bundle_component(from_path);
    if !normalized.is_empty() {
        return normalized;
    }

    original_bundle_id
        .split('.')
        .next_back()
        .map(normalize_bundle_component)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "app".into())
}

fn generate_temp_bundle_id(
    config: &Config,
    app_path: &Path,
    original_bundle_id: &str,
) -> Result<String> {
    let prefix = temp_bundle_id_prefix(config)?;
    let app_name = display_app_name(app_path, original_bundle_id);
    let suffix = stable_hex_suffix(original_bundle_id);
    Ok(format!(
        "{}.{}.{}",
        prefix.trim_matches('.'),
        app_name,
        suffix
    ))
}

fn profiles_created_for_bundle_id(
    before_paths: &[PathBuf],
    after_profiles: &[ProvisioningProfile],
    bundle_id: &str,
) -> Vec<PathBuf> {
    after_profiles
        .iter()
        .filter(|profile| profile.bundle_id_pattern == bundle_id)
        .filter(|profile| !before_paths.iter().any(|path| path == &profile.path))
        .map(|profile| profile.path.clone())
        .collect()
}

const PROJECT_PBXPROJ_TEMPLATE: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 56;
	objects = {

/* Begin PBXBuildFile section */
		AA000001 /* main.swift in Sources */ = {isa = PBXBuildFile; fileRef = AA000002 /* main.swift */; };
/* End PBXBuildFile section */

/* Begin PBXFileReference section */
		AA000002 /* main.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = main.swift; sourceTree = "<group>"; };
		AA000003 /* App.app */ = {isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = App.app; sourceTree = BUILT_PRODUCTS_DIR; };
/* End PBXFileReference section */

/* Begin PBXFrameworksBuildPhase section */
		AA000004 /* Frameworks */ = {
			isa = PBXFrameworksBuildPhase;
			buildActionMask = 2147483647;
			files = (
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
/* End PBXFrameworksBuildPhase section */

/* Begin PBXGroup section */
		AA000005 = {
			isa = PBXGroup;
			children = (
				AA000002 /* main.swift */,
				AA000006 /* Products */,
			);
			sourceTree = "<group>";
		};
		AA000006 /* Products */ = {
			isa = PBXGroup;
			children = (
				AA000003 /* App.app */,
			);
			name = Products;
			sourceTree = "<group>";
		};
/* End PBXGroup section */

/* Begin PBXNativeTarget section */
		AA000007 /* App */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = AA000008 /* Build configuration list for PBXNativeTarget "App" */;
			buildPhases = (
				AA000009 /* Sources */,
				AA000004 /* Frameworks */,
			);
			buildRules = (
			);
			dependencies = (
			);
			name = App;
			productName = App;
			productReference = AA000003 /* App.app */;
			productType = "com.apple.product-type.application";
		};
/* End PBXNativeTarget section */

/* Begin PBXProject section */
		AA000010 /* Project object */ = {
			isa = PBXProject;
			attributes = {
				LastUpgradeCheck = 1540;
			};
			buildConfigurationList = AA000011 /* Build configuration list for PBXProject "App" */;
			compatibilityVersion = "Xcode 14.0";
			developmentRegion = en;
			hasScannedForEncodings = 0;
			knownRegions = (
				en,
				Base,
			);
			mainGroup = AA000005;
			productRefGroup = AA000006 /* Products */;
			projectDirPath = "";
			projectRoot = "";
			targets = (
				AA000007 /* App */,
			);
		};
/* End PBXProject section */

/* Begin PBXSourcesBuildPhase section */
		AA000009 /* Sources */ = {
			isa = PBXSourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
				AA000001 /* main.swift in Sources */,
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
/* End PBXSourcesBuildPhase section */

/* Begin XCBuildConfiguration section */
		AA000012 /* Debug */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				CODE_SIGN_STYLE = Automatic;
				DEVELOPMENT_TEAM = __TEAM_ID__;
				GENERATE_INFOPLIST_FILE = YES;
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				PRODUCT_BUNDLE_IDENTIFIER = __BUNDLE_ID__;
				PRODUCT_NAME = "$(TARGET_NAME)";
				SDKROOT = iphoneos;
				SUPPORTED_PLATFORMS = "iphoneos iphonesimulator";
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "1,2";
			};
			name = Debug;
		};
		AA000013 /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				CODE_SIGN_STYLE = Automatic;
				DEVELOPMENT_TEAM = __TEAM_ID__;
				GENERATE_INFOPLIST_FILE = YES;
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				PRODUCT_BUNDLE_IDENTIFIER = __BUNDLE_ID__;
				PRODUCT_NAME = "$(TARGET_NAME)";
				SDKROOT = iphoneos;
				SUPPORTED_PLATFORMS = "iphoneos iphonesimulator";
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "1,2";
			};
			name = Release;
		};
		AA000014 /* Debug */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
			};
			name = Debug;
		};
		AA000015 /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
			};
			name = Release;
		};
/* End XCBuildConfiguration section */

/* Begin XCConfigurationList section */
		AA000008 /* Build configuration list for PBXNativeTarget "App" */ = {
			isa = XCConfigurationList;
			buildConfigurations = (
				AA000012 /* Debug */,
				AA000013 /* Release */,
			);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		};
		AA000011 /* Build configuration list for PBXProject "App" */ = {
			isa = XCConfigurationList;
			buildConfigurations = (
				AA000014 /* Debug */,
				AA000015 /* Release */,
			);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		};
/* End XCConfigurationList section */

	};
	rootObject = AA000010 /* Project object */;
}
"#;

fn create_minimal_xcode_project(dir: &Path, bundle_id: &str, team_id: &str) -> Result<()> {
    let proj_dir = dir.join("App.xcodeproj");
    fs::create_dir_all(&proj_dir)?;
    let pbxproj = PROJECT_PBXPROJ_TEMPLATE
        .replace("__BUNDLE_ID__", bundle_id)
        .replace("__TEAM_ID__", team_id);
    fs::write(proj_dir.join("project.pbxproj"), pbxproj)?;
    fs::write(dir.join("main.swift"), "import UIKit\n")?;
    Ok(())
}

fn auto_provision(bundle_id: &str, identity: &SigningIdentity, device_udid: &str) -> Result<()> {
    let team_id = extract_team_id(identity).ok_or_else(|| {
        TossError::Signing("cannot parse team ID from signing identity name".into())
    })?;

    let temp = TempDir::new()?;
    create_minimal_xcode_project(temp.path(), bundle_id, &team_id)?;

    let project_path = temp.path().join("App.xcodeproj");
    xcrun::build_for_device(&project_path, false, "App", device_udid, false).map_err(|err| {
        TossError::Signing(format!(
            "auto-provisioning failed for bundle ID '{}': {}",
            bundle_id, err
        ))
    })?;

    Ok(())
}

fn resolve_signing_plan(
    config: &Config,
    extracted: &ExtractedApp,
    identity: &SigningIdentity,
    device_udid: &str,
    profile_override: Option<&str>,
) -> Result<SigningPlan> {
    if let Some(path) = profile_override {
        let profile = load_profile_from_path(Path::new(path))?;
        return Ok(SigningPlan {
            profile,
            final_bundle_id: extracted.bundle_id.clone(),
            cleanup_profiles: Vec::new(),
        });
    }

    if let Ok(profile) = find_provisioning_profiles()
        .and_then(|profiles| match_profile(&profiles, &extracted.bundle_id))
    {
        return Ok(SigningPlan {
            profile,
            final_bundle_id: extracted.bundle_id.clone(),
            cleanup_profiles: Vec::new(),
        });
    }

    ensure_no_app_extensions(&extracted.app_path)?;

    let temp_bundle_id =
        generate_temp_bundle_id(config, &extracted.app_path, &extracted.bundle_id)?;
    println!(
        "No usable profile for '{}' found, switching to temporary bundle ID '{}'.",
        extracted.bundle_id, temp_bundle_id
    );

    let profiles_before = find_provisioning_profiles().unwrap_or_default();
    if let Ok(profile) = match_profile(&profiles_before, &temp_bundle_id) {
        return Ok(SigningPlan {
            profile,
            final_bundle_id: temp_bundle_id,
            cleanup_profiles: Vec::new(),
        });
    }

    println!(
        "Auto-provisioning temporary bundle ID '{}'...",
        temp_bundle_id
    );
    let before_paths = list_profile_paths(&profiles_before);
    auto_provision(&temp_bundle_id, identity, device_udid)?;

    let profiles_after = find_provisioning_profiles()?;
    let profile = match_profile(&profiles_after, &temp_bundle_id)?;
    let cleanup_profiles =
        profiles_created_for_bundle_id(&before_paths, &profiles_after, &temp_bundle_id);

    Ok(SigningPlan {
        profile,
        final_bundle_id: temp_bundle_id,
        cleanup_profiles,
    })
}

// ---------------------------------------------------------------------------
// Top-level workflow
// ---------------------------------------------------------------------------

pub fn sign_workflow(
    config: &Config,
    ipa_path: &Path,
    device_id: &str,
    device_udid: &str,
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

    let signing_plan =
        resolve_signing_plan(config, &extracted, &identity, device_udid, profile_override)?;
    println!("Profile: {}", signing_plan.profile.name);

    let result = (|| {
        if signing_plan.final_bundle_id != extracted.bundle_id {
            rewrite_bundle_id(&extracted.app_path, &signing_plan.final_bundle_id)?;
            println!(
                "Bundle ID: {} → {}",
                extracted.bundle_id, signing_plan.final_bundle_id
            );
        }

        // 4. Replace embedded.mobileprovision
        let embedded = extracted.app_path.join("embedded.mobileprovision");
        fs::copy(&signing_plan.profile.path, &embedded)?;

        // 5. Extract entitlements from profile
        let ent_path =
            extract_entitlements(&signing_plan.profile.path, extracted._temp_dir.path())?;

        // 6. Re-sign
        resign_app(&extracted.app_path, &identity, &ent_path)?;

        // 7. Install
        xcrun::install_app(device_id, &extracted.app_path)?;

        // 8. Optionally launch
        if launch {
            xcrun::launch_app(device_id, &signing_plan.final_bundle_id)?;
        }

        Ok(())
    })();

    match cleanup_profiles(&signing_plan.cleanup_profiles) {
        Ok(()) if !signing_plan.cleanup_profiles.is_empty() => {
            println!(
                "Cleaned {} temporary provisioning profile(s).",
                signing_plan.cleanup_profiles.len()
            );
        }
        Ok(()) => {}
        Err(err) => eprintln!("warning: failed to clean temporary profiles: {}", err),
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{Config, SigningConfig};

    #[test]
    fn temp_bundle_id_is_stable_and_prefixed() {
        let config = Config {
            signing: SigningConfig {
                temp_bundle_prefix: Some("cn.yangym.tmp".into()),
            },
            ..Config::default()
        };

        let app_path = Path::new("/tmp/Kazumi.app");
        let one = generate_temp_bundle_id(&config, app_path, "com.example.kazumi").unwrap();
        let two = generate_temp_bundle_id(&config, app_path, "com.example.kazumi").unwrap();

        assert_eq!(one, two);
        assert!(one.starts_with("cn.yangym.tmp.kazumi."));
    }

    #[test]
    fn created_profiles_only_include_new_exact_matches() {
        let before = vec![PathBuf::from("/tmp/existing.mobileprovision")];
        let after = vec![
            ProvisioningProfile {
                path: PathBuf::from("/tmp/existing.mobileprovision"),
                name: "existing".into(),
                team_ids: vec![],
                bundle_id_pattern: "cn.yangym.tmp.kazumi.1234".into(),
            },
            ProvisioningProfile {
                path: PathBuf::from("/tmp/new.mobileprovision"),
                name: "new".into(),
                team_ids: vec![],
                bundle_id_pattern: "cn.yangym.tmp.kazumi.1234".into(),
            },
            ProvisioningProfile {
                path: PathBuf::from("/tmp/wildcard.mobileprovision"),
                name: "wildcard".into(),
                team_ids: vec![],
                bundle_id_pattern: "cn.yangym.tmp.*".into(),
            },
        ];

        let created = profiles_created_for_bundle_id(&before, &after, "cn.yangym.tmp.kazumi.1234");
        assert_eq!(created, vec![PathBuf::from("/tmp/new.mobileprovision")]);
    }
}
