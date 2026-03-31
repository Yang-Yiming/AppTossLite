use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use tempfile::{NamedTempFile, TempDir};

use super::config::Config;
use super::error::{Result, TossError};
use super::interaction::{WorkflowAdapter, WorkflowEvent, choose_index};
use super::project::extract_bundle_id;
use super::xcrun;

#[derive(Debug, Clone)]
pub struct SigningIdentity {
    pub hash: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ProvisioningProfile {
    pub path: PathBuf,
    pub uuid: Option<String>,
    pub name: String,
    pub team_ids: Vec<String>,
    pub bundle_id_pattern: String,
    pub expiration_epoch: Option<u64>,
    pub provisioned_devices: Vec<String>,
    pub provisions_all_devices: bool,
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
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BundleKind {
    App,
    AppExtension,
}

impl BundleKind {
    fn display_name(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::AppExtension => "app extension",
        }
    }
}

#[derive(Debug, Clone)]
struct BundleTarget {
    path: PathBuf,
    kind: BundleKind,
    original_bundle_id: String,
    final_bundle_id: String,
    profile: ProvisioningProfile,
}

struct SigningPlan {
    targets: Vec<BundleTarget>,
    cleanup_profiles: Vec<PathBuf>,
}

pub struct SignOutcome {
    pub app_path: PathBuf,
    pub final_bundle_id: String,
    pub launched: bool,
}

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

    let payload_dir = temp_dir.path().join("Payload");
    if !payload_dir.is_dir() {
        return Err(TossError::Signing("IPA has no Payload/ directory".into()));
    }

    let app_path = fs::read_dir(&payload_dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "app"))
        .map(|e| e.path())
        .ok_or_else(|| TossError::Signing("no .app found in Payload/".into()))?;

    Ok(ExtractedApp {
        _temp_dir: temp_dir,
        app_path,
    })
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

fn rewrite_bundle_id(bundle_path: &Path, bundle_id: &str) -> Result<()> {
    let info_plist = bundle_path.join("Info.plist");
    plist_set_string(&info_plist, "CFBundleIdentifier", bundle_id)
}

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
        if let Some(rest) = trimmed.strip_prefix(|c: char| c.is_ascii_digit()) {
            let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == ')');
            let rest = rest.trim();
            if rest.len() >= 40 && rest[..40].chars().all(|c| c.is_ascii_hexdigit()) {
                let hash = rest[..40].to_string();
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
    adapter: &mut impl WorkflowAdapter,
) -> Result<SigningIdentity> {
    if let Some(query) = override_name {
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
                let selection = choose_index(
                    adapter,
                    "Multiple identities match, select one",
                    &items.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
                    TossError::Signing(
                        "multiple signing identities match — specify one with `--identity`".into(),
                    ),
                )?;
                Ok(matches[selection].clone())
            }
        };
    }

    match identities.len() {
        1 => Ok(identities[0].clone()),
        _ => {
            let items: Vec<&str> = identities.iter().map(|id| id.name.as_str()).collect();
            let selection = choose_index(
                adapter,
                "Select signing identity",
                &items.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
                TossError::Signing(
                    "multiple signing identities found — specify one with `--identity`".into(),
                ),
            )?;
            Ok(identities[selection].clone())
        }
    }
}

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
    let tmp = decode_provisioning_profile(path)?;

    let name = plutil_extract(tmp.path(), "Name").unwrap_or_else(|| "Unknown".into());
    let uuid = plutil_extract(tmp.path(), "UUID");

    let mut team_ids = Vec::new();
    for i in 0..32 {
        match plutil_extract(tmp.path(), &format!("TeamIdentifier.{}", i)) {
            Some(id) => team_ids.push(id),
            None => break,
        }
    }

    let app_id =
        plutil_extract(tmp.path(), "Entitlements.application-identifier").unwrap_or_default();
    let bundle_id_pattern = app_id
        .find('.')
        .map(|i| app_id[i + 1..].to_string())
        .unwrap_or(app_id);

    let expiration_epoch = plutil_extract(tmp.path(), "ExpirationDate")
        .and_then(|value| parse_profile_date_to_epoch(&value));

    let mut provisioned_devices = Vec::new();
    for i in 0..512 {
        match plutil_extract(tmp.path(), &format!("ProvisionedDevices.{}", i)) {
            Some(device) => provisioned_devices.push(device),
            None => break,
        }
    }

    let provisions_all_devices = plutil_extract(tmp.path(), "ProvisionsAllDevices")
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(false);

    Ok(ProvisioningProfile {
        path: path.to_path_buf(),
        uuid,
        name,
        team_ids,
        bundle_id_pattern,
        expiration_epoch,
        provisioned_devices,
        provisions_all_devices,
    })
}

fn decode_provisioning_profile(path: &Path) -> Result<NamedTempFile> {
    let cms_output = Command::new("security")
        .args(["cms", "-D", "-i"])
        .arg(path)
        .output()?;

    let tmp = NamedTempFile::new()?;
    if cms_output.status.success() {
        fs::write(tmp.path(), &cms_output.stdout)?;
        return Ok(tmp);
    }

    let raw = fs::read(path)?;
    if let Some(plist) = extract_embedded_plist_bytes(&raw) {
        fs::write(tmp.path(), plist)?;
        return Ok(tmp);
    }

    return Err(TossError::Signing(format!(
        "failed to decode provisioning profile {}: {}",
        path.display(),
        String::from_utf8_lossy(&cms_output.stderr).trim()
    )));
}

fn extract_embedded_plist_bytes(raw: &[u8]) -> Option<&[u8]> {
    let start = find_bytes(raw, b"<?xml").or_else(|| find_bytes(raw, b"<plist"))?;
    let end_start = find_bytes(&raw[start..], b"</plist>")?;
    let end = start + end_start + b"</plist>".len();
    Some(&raw[start..end])
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }

    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn identity_matches_profile(identity: &SigningIdentity, profile: &ProvisioningProfile) -> bool {
    if profile.team_ids.is_empty() {
        return true;
    }

    profile.team_ids.iter().any(|team_id| {
        identity.name.contains(&format!("({})", team_id)) || identity.name.contains(team_id)
    })
}

fn parse_profile_date_to_epoch(raw: &str) -> Option<u64> {
    for format in ["%Y-%m-%d %H:%M:%S %z", "%Y-%m-%dT%H:%M:%SZ"] {
        let output = Command::new("date")
            .args(["-j", "-u", "-f", format, raw, "+%s"])
            .output()
            .ok()?;
        if output.status.success() {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Ok(epoch) = value.parse::<u64>() {
                return Some(epoch);
            }
        }
    }
    None
}

fn profile_matches_bundle_id(profile: &ProvisioningProfile, bundle_id: &str) -> bool {
    if profile.bundle_id_pattern == "*" {
        return true;
    }
    if let Some(prefix) = profile.bundle_id_pattern.strip_suffix('*') {
        return bundle_id.starts_with(prefix);
    }
    profile.bundle_id_pattern == bundle_id
}

fn profile_compatibility_issues(
    profile: &ProvisioningProfile,
    _identity: &SigningIdentity,
    bundle_id: &str,
    device_udid: &str,
) -> Vec<String> {
    let mut issues = Vec::new();

    if !profile_matches_bundle_id(profile, bundle_id) {
        issues.push(format!(
            "bundle id '{}' does not match profile pattern '{}'",
            bundle_id, profile.bundle_id_pattern
        ));
    }

    if let Some(expiration_epoch) = profile.expiration_epoch {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        if expiration_epoch <= now {
            issues.push("profile is expired".into());
        }
    }

    if !profile.provisions_all_devices
        && !profile.provisioned_devices.is_empty()
        && !profile
            .provisioned_devices
            .iter()
            .any(|udid| udid == device_udid)
    {
        issues.push(format!("profile does not include device {}", device_udid));
    }

    issues
}

fn match_compatible_profile(
    profiles: &[ProvisioningProfile],
    bundle_id: &str,
    identity: &SigningIdentity,
    device_udid: &str,
    adapter: &mut impl WorkflowAdapter,
) -> Result<Option<ProvisioningProfile>> {
    let exact: Vec<_> = profiles
        .iter()
        .filter(|p| p.bundle_id_pattern == bundle_id)
        .filter(|p| profile_compatibility_issues(p, identity, bundle_id, device_udid).is_empty())
        .filter(|p| identity_matches_profile(identity, p))
        .collect();

    if exact.len() == 1 {
        return Ok(Some(exact[0].clone()));
    }

    let wildcard: Vec<_> = profiles
        .iter()
        .filter(|p| profile_matches_bundle_id(p, bundle_id))
        .filter(|p| profile_compatibility_issues(p, identity, bundle_id, device_udid).is_empty())
        .filter(|p| identity_matches_profile(identity, p))
        .collect();

    let candidates = if !exact.is_empty() { &exact } else { &wildcard };

    match candidates.len() {
        0 => Ok(None),
        1 => Ok(Some(candidates[0].clone())),
        _ => {
            let items: Vec<String> = candidates
                .iter()
                .map(|p| {
                    let uuid = p.uuid.as_deref().unwrap_or("no-uuid");
                    format!("{} ({}, {})", p.name, p.bundle_id_pattern, uuid)
                })
                .collect();
            let selection = choose_index(
                adapter,
                "Multiple compatible profiles match, select one",
                &items,
                TossError::Signing(
                    "multiple compatible provisioning profiles found — specify one with `--profile`"
                        .into(),
                ),
            )?;
            Ok(Some(candidates[selection].clone()))
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

fn temp_team_id(config: &Config) -> Result<&str> {
    let team_id = config.signing.team_id.as_deref().ok_or_else(|| {
        TossError::Signing("temporary signing requires `toss config set-team-id <TEAMID>`".into())
    })?;

    if !team_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(TossError::Signing(format!(
            "invalid configured team id '{}' — update it with `toss config set-team-id <TEAMID>`",
            team_id
        )));
    }

    Ok(team_id)
}

fn display_app_name(bundle_path: &Path, original_bundle_id: &str) -> String {
    let from_path = bundle_path
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
    bundle_path: &Path,
    original_bundle_id: &str,
) -> Result<String> {
    let prefix = temp_bundle_id_prefix(config)?;
    let app_name = display_app_name(bundle_path, original_bundle_id);
    let suffix = stable_hex_suffix(original_bundle_id);
    Ok(format!(
        "{}.{}.{}",
        prefix.trim_matches('.'),
        app_name,
        suffix
    ))
}

fn derive_extension_bundle_id(
    main_original_bundle_id: &str,
    main_final_bundle_id: &str,
    extension_original_bundle_id: &str,
    extension_path: &Path,
    config: &Config,
) -> Result<String> {
    if let Some(suffix) = extension_original_bundle_id.strip_prefix(main_original_bundle_id)
        && suffix.starts_with('.')
    {
        return Ok(format!("{}{}", main_final_bundle_id, suffix));
    }

    generate_temp_bundle_id(config, extension_path, extension_original_bundle_id)
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
		AA000001 /* App.swift in Sources */ = {isa = PBXBuildFile; fileRef = AA000002 /* App.swift */; };
/* End PBXBuildFile section */

/* Begin PBXFileReference section */
		AA000002 /* App.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = App.swift; sourceTree = "<group>"; };
		AA000016 /* Info.plist */ = {isa = PBXFileReference; lastKnownFileType = text.plist.xml; path = Info.plist; sourceTree = "<group>"; };
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
				AA000002 /* App.swift */,
				AA000016 /* Info.plist */,
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
				BuildIndependentTargetsInParallel = 1;
				LastUpgradeCheck = 1540;
				LastSwiftUpdateCheck = 1540;
				TargetAttributes = {
					AA000007 = {
						CreatedOnToolsVersion = 15.0;
					};
				};
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
				ALWAYS_SEARCH_USER_PATHS = NO;
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				DEVELOPMENT_TEAM = __TEAM_ID__;
				GENERATE_INFOPLIST_FILE = NO;
				INFOPLIST_FILE = Info.plist;
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				LD_RUNPATH_SEARCH_PATHS = "@executable_path/Frameworks";
				MARKETING_VERSION = 1.0;
				PRODUCT_BUNDLE_IDENTIFIER = __BUNDLE_ID__;
				PRODUCT_NAME = "$(TARGET_NAME)";
				SDKROOT = iphoneos;
				SUPPORTED_PLATFORMS = iphoneos;
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "1,2";
			};
			name = Debug;
		};
		AA000013 /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				ALWAYS_SEARCH_USER_PATHS = NO;
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				DEVELOPMENT_TEAM = __TEAM_ID__;
				GENERATE_INFOPLIST_FILE = NO;
				INFOPLIST_FILE = Info.plist;
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				LD_RUNPATH_SEARCH_PATHS = "@executable_path/Frameworks";
				MARKETING_VERSION = 1.0;
				PRODUCT_BUNDLE_IDENTIFIER = __BUNDLE_ID__;
				PRODUCT_NAME = "$(TARGET_NAME)";
				SDKROOT = iphoneos;
				SUPPORTED_PLATFORMS = iphoneos;
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "1,2";
			};
			name = Release;
		};
		AA000014 /* Debug */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				SDKROOT = iphoneos;
			};
			name = Debug;
		};
		AA000015 /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				SDKROOT = iphoneos;
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
    fs::write(
        dir.join("App.swift"),
        r#"import SwiftUI

@main
struct ProvisioningProbeApp: App {
    var body: some Scene {
        WindowGroup {
            Text("Provisioning Probe")
        }
    }
}
"#,
    )?;
    fs::write(
        dir.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>UIApplicationSceneManifest</key>
	<dict>
		<key>UIApplicationSupportsMultipleScenes</key>
		<false/>
	</dict>
</dict>
</plist>
"#,
    )?;
    Ok(())
}

fn auto_provision(bundle_id: &str, team_id: &str, device_udid: &str) -> Result<()> {
    let temp = TempDir::new()?;
    create_minimal_xcode_project(temp.path(), bundle_id, team_id)?;

    let project_path = temp.path().join("App.xcodeproj");
    xcrun::build_for_device(&project_path, false, "App", device_udid, false).map_err(|err| {
        TossError::Signing(format!(
            "auto-provisioning failed for bundle ID '{}': {}",
            bundle_id, err
        ))
    })?;

    Ok(())
}

fn scan_signing_targets(app_path: &Path) -> Result<Vec<(PathBuf, BundleKind, String)>> {
    let mut targets = vec![(
        app_path.to_path_buf(),
        BundleKind::App,
        extract_bundle_id(app_path)?,
    )];
    collect_app_extensions(app_path, &mut targets)?;
    Ok(targets)
}

fn collect_app_extensions(
    bundle_path: &Path,
    targets: &mut Vec<(PathBuf, BundleKind, String)>,
) -> Result<()> {
    let plugins_dir = bundle_path.join("PlugIns");
    if !plugins_dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(&plugins_dir)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "appex") {
            let bundle_id = extract_bundle_id(&path)?;
            targets.push((path.clone(), BundleKind::AppExtension, bundle_id));
            collect_app_extensions(&path, targets)?;
        }
    }

    Ok(())
}

fn list_profile_paths(profiles: &[ProvisioningProfile]) -> Vec<PathBuf> {
    profiles.iter().map(|p| p.path.clone()).collect()
}

fn resolve_bundle_target(
    config: &Config,
    available_profiles: &mut Vec<ProvisioningProfile>,
    cleanup_profiles: &mut Vec<PathBuf>,
    identity: &SigningIdentity,
    device_udid: &str,
    bundle_path: &Path,
    kind: BundleKind,
    original_bundle_id: &str,
    preferred_bundle_id: &str,
    profile_override: Option<&Path>,
    adapter: &mut impl WorkflowAdapter,
) -> Result<BundleTarget> {
    if let Some(path) = profile_override {
        let profile = load_profile_from_path(path)?;
        let issues =
            profile_compatibility_issues(&profile, identity, original_bundle_id, device_udid);
        if !issues.is_empty() {
            return Err(TossError::Signing(format!(
                "override profile '{}' is incompatible with {} '{}': {}",
                path.display(),
                kind.display_name(),
                original_bundle_id,
                issues.join("; ")
            )));
        }
        return Ok(BundleTarget {
            path: bundle_path.to_path_buf(),
            kind,
            original_bundle_id: original_bundle_id.to_string(),
            final_bundle_id: original_bundle_id.to_string(),
            profile,
        });
    }

    if let Some(profile) = match_compatible_profile(
        available_profiles,
        original_bundle_id,
        identity,
        device_udid,
        adapter,
    )? {
        return Ok(BundleTarget {
            path: bundle_path.to_path_buf(),
            kind,
            original_bundle_id: original_bundle_id.to_string(),
            final_bundle_id: original_bundle_id.to_string(),
            profile,
        });
    }

    let final_bundle_id = preferred_bundle_id.to_string();

    if let Some(profile) = match_compatible_profile(
        available_profiles,
        &final_bundle_id,
        identity,
        device_udid,
        adapter,
    )? {
        return Ok(BundleTarget {
            path: bundle_path.to_path_buf(),
            kind,
            original_bundle_id: original_bundle_id.to_string(),
            final_bundle_id,
            profile,
        });
    }

    let team_id = temp_team_id(config)?;
    let before_paths = list_profile_paths(available_profiles);
    adapter.emit(WorkflowEvent::AutoProvisioning {
        kind: kind.display_name().to_string(),
        bundle_id: final_bundle_id.clone(),
        device_udid: device_udid.to_string(),
    })?;
    auto_provision(&final_bundle_id, team_id, device_udid)?;

    *available_profiles = find_provisioning_profiles()?;
    let profile = match_compatible_profile(
        available_profiles,
        &final_bundle_id,
        identity,
        device_udid,
        adapter,
    )?
    .ok_or_else(|| {
        TossError::Signing(format!(
            "auto-provisioning finished but no compatible profile was found for {} '{}'",
            kind.display_name(),
            final_bundle_id
        ))
    })?;

    cleanup_profiles.extend(profiles_created_for_bundle_id(
        &before_paths,
        available_profiles,
        &final_bundle_id,
    ));

    Ok(BundleTarget {
        path: bundle_path.to_path_buf(),
        kind,
        original_bundle_id: original_bundle_id.to_string(),
        final_bundle_id,
        profile,
    })
}

fn resolve_signing_plan(
    config: &Config,
    extracted: &ExtractedApp,
    identity: &SigningIdentity,
    device_udid: &str,
    profile_override: Option<&str>,
    adapter: &mut impl WorkflowAdapter,
) -> Result<SigningPlan> {
    let discovered = scan_signing_targets(&extracted.app_path)?;
    let mut available_profiles = find_provisioning_profiles().unwrap_or_default();
    let mut cleanup_profiles = Vec::new();
    let override_path = profile_override.map(Path::new);

    let (main_path, main_kind, main_original_bundle_id) = discovered
        .first()
        .cloned()
        .ok_or_else(|| TossError::Signing("no app bundle discovered after extraction".into()))?;

    let main_preferred_bundle_id = if match_compatible_profile(
        &available_profiles,
        &main_original_bundle_id,
        identity,
        device_udid,
        adapter,
    )?
    .is_some()
        || override_path.is_some()
    {
        main_original_bundle_id.clone()
    } else {
        let generated = generate_temp_bundle_id(config, &main_path, &main_original_bundle_id)?;
        adapter.emit(WorkflowEvent::TemporaryBundleId {
            original_bundle_id: main_original_bundle_id.clone(),
            temporary_bundle_id: generated.clone(),
        })?;
        generated
    };

    let mut targets = Vec::new();
    let main_target = resolve_bundle_target(
        config,
        &mut available_profiles,
        &mut cleanup_profiles,
        identity,
        device_udid,
        &main_path,
        main_kind,
        &main_original_bundle_id,
        &main_preferred_bundle_id,
        override_path,
        adapter,
    )?;
    let main_final_bundle_id = main_target.final_bundle_id.clone();
    targets.push(main_target);

    for (path, kind, original_bundle_id) in discovered.into_iter().skip(1) {
        let preferred_bundle_id = if match_compatible_profile(
            &available_profiles,
            &original_bundle_id,
            identity,
            device_udid,
            adapter,
        )?
        .is_some()
        {
            original_bundle_id.clone()
        } else {
            derive_extension_bundle_id(
                &main_original_bundle_id,
                &main_final_bundle_id,
                &original_bundle_id,
                &path,
                config,
            )?
        };

        let target = resolve_bundle_target(
            config,
            &mut available_profiles,
            &mut cleanup_profiles,
            identity,
            device_udid,
            &path,
            kind,
            &original_bundle_id,
            &preferred_bundle_id,
            None,
            adapter,
        )?;
        targets.push(target);
    }

    let mut seen = HashSet::new();
    cleanup_profiles.retain(|path| seen.insert(path.clone()));

    Ok(SigningPlan {
        targets,
        cleanup_profiles,
    })
}

fn extract_profile_entitlements(
    profile_path: &Path,
    temp_dir: &Path,
    name: &str,
) -> Result<PathBuf> {
    let decoded = decode_provisioning_profile(profile_path)?;
    let ent_path = temp_dir.join(format!("{}_profile_entitlements.plist", name));
    let output = Command::new("plutil")
        .args([
            "-extract",
            "Entitlements",
            "xml1",
            "-o",
            &ent_path.to_string_lossy(),
            &decoded.path().to_string_lossy(),
        ])
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "failed to extract entitlements from {}: {}",
            profile_path.display(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(ent_path)
}

fn dump_existing_entitlements(
    bundle_path: &Path,
    temp_dir: &Path,
    name: &str,
) -> Result<Option<PathBuf>> {
    let output = Command::new("codesign")
        .args(["-d", "--entitlements", ":-"])
        .arg(bundle_path)
        .output()?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let start = combined.find("<?xml").or_else(|| combined.find("<plist"));
    let end = combined.rfind("</plist>");

    match (start, end) {
        (Some(start), Some(end)) if start < end => {
            let ent_path = temp_dir.join(format!("{}_original_entitlements.plist", name));
            fs::write(&ent_path, &combined[start..end + "</plist>".len()])?;
            Ok(Some(ent_path))
        }
        _ => Ok(None),
    }
}

fn plist_to_json_value(plist_path: &Path) -> Result<Value> {
    let output = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(plist_path)
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "failed to convert {} to json: {}",
            plist_path.display(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

fn json_value_to_plist(value: &Value, output_path: &Path) -> Result<()> {
    let json_file = NamedTempFile::new()?;
    fs::write(json_file.path(), serde_json::to_vec(value)?)?;

    let output = Command::new("plutil")
        .args([
            "-convert",
            "xml1",
            "-o",
            &output_path.to_string_lossy(),
            &json_file.path().to_string_lossy(),
        ])
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "failed to convert entitlements json to plist: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}

fn required_profile_entitlement_keys() -> [&'static str; 5] {
    [
        "application-identifier",
        "com.apple.developer.team-identifier",
        "keychain-access-groups",
        "get-task-allow",
        "aps-environment",
    ]
}

fn merged_entitlements(
    profile_entitlements: &Value,
    original_entitlements: Option<&Value>,
) -> Value {
    let profile_obj = match profile_entitlements.as_object() {
        Some(obj) => obj,
        None => return profile_entitlements.clone(),
    };

    let original_obj = original_entitlements.and_then(Value::as_object);
    let mut merged = Map::new();

    if let Some(original_obj) = original_obj {
        for key in original_obj.keys() {
            if let Some(value) = profile_obj.get(key) {
                merged.insert(key.clone(), value.clone());
            }
        }
    }

    for key in required_profile_entitlement_keys() {
        if let Some(value) = profile_obj.get(key) {
            merged.insert(key.to_string(), value.clone());
        }
    }

    if merged.is_empty() {
        Value::Object(profile_obj.clone())
    } else {
        Value::Object(merged)
    }
}

fn create_codesign_entitlements(
    target: &BundleTarget,
    temp_dir: &Path,
    name: &str,
) -> Result<PathBuf> {
    let profile_entitlements = extract_profile_entitlements(&target.profile.path, temp_dir, name)?;
    let profile_json = plist_to_json_value(&profile_entitlements)?;
    let original_json = dump_existing_entitlements(&target.path, temp_dir, name)?
        .map(|path| plist_to_json_value(&path))
        .transpose()?;
    let merged = merged_entitlements(&profile_json, original_json.as_ref());
    let final_path = temp_dir.join(format!("{}_codesign_entitlements.plist", name));
    json_value_to_plist(&merged, &final_path)?;
    Ok(final_path)
}

fn sign_frameworks(bundle_path: &Path, identity: &SigningIdentity) -> Result<()> {
    let frameworks_dir = bundle_path.join("Frameworks");
    if !frameworks_dir.is_dir() {
        return Ok(());
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(&frameworks_dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    entries.sort();

    for path in entries {
        if path.extension().is_some_and(|ext| ext == "framework") {
            sign_frameworks(&path, identity)?;
            codesign_path(&path, identity, None)?;
        } else if path.extension().is_some_and(|ext| ext == "dylib") {
            codesign_path(&path, identity, None)?;
        }
    }

    Ok(())
}

fn codesign_path(
    path: &Path,
    identity: &SigningIdentity,
    entitlements: Option<&Path>,
) -> Result<()> {
    let mut command = Command::new("codesign");
    command.args(["--force", "--sign", &identity.hash]);
    if let Some(entitlements) = entitlements {
        command.args(["--entitlements", &entitlements.to_string_lossy()]);
    }
    command.arg(path);

    let output = command.output()?;
    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "codesign failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}

fn verify_signed_app(app_path: &Path) -> Result<()> {
    let output = Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app_path)
        .output()?;

    if !output.status.success() {
        return Err(TossError::Signing(format!(
            "codesign verification failed for {}: {}",
            app_path.display(),
            String::from_utf8_lossy(&output.stderr)
        )));
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

pub fn sign_workflow(
    config: &Config,
    ipa_path: &Path,
    device_id: &str,
    device_udid: &str,
    identity_override: Option<&str>,
    profile_override: Option<&str>,
    launch: bool,
    adapter: &mut impl WorkflowAdapter,
) -> Result<SignOutcome> {
    let extracted = unzip_ipa(ipa_path)?;
    let extracted_bundle_id = extract_bundle_id(&extracted.app_path)?;
    adapter.emit(WorkflowEvent::ExtractedBundle {
        bundle_id: extracted_bundle_id,
        app_name: extracted
            .app_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
    })?;

    let identities = list_signing_identities()?;
    let identity = select_signing_identity(&identities, identity_override, adapter)?;
    adapter.emit(WorkflowEvent::UsingIdentity {
        identity_name: identity.name.clone(),
    })?;

    let signing_plan = resolve_signing_plan(
        config,
        &extracted,
        &identity,
        device_udid,
        profile_override,
        adapter,
    )?;

    for target in &signing_plan.targets {
        adapter.emit(WorkflowEvent::SigningPlanStep {
            kind: target.kind.display_name().to_string(),
            original_bundle_id: target.original_bundle_id.clone(),
            final_bundle_id: target.final_bundle_id.clone(),
            profile_name: target.profile.name.clone(),
        })?;
    }

    let main_bundle_id = signing_plan
        .targets
        .iter()
        .find(|target| target.path == extracted.app_path)
        .map(|target| target.final_bundle_id.clone())
        .ok_or_else(|| TossError::Signing("main app target missing from signing plan".into()))?;

    let app_path = extracted.app_path.clone();
    let cleanup_paths = signing_plan.cleanup_profiles.clone();

    let result = (|| -> Result<SignOutcome> {
        for target in &signing_plan.targets {
            if target.final_bundle_id != target.original_bundle_id {
                rewrite_bundle_id(&target.path, &target.final_bundle_id)?;
                adapter.emit(WorkflowEvent::BundleIdRewritten {
                    from: target.original_bundle_id.clone(),
                    to: target.final_bundle_id.clone(),
                })?;
            }

            let embedded = target.path.join("embedded.mobileprovision");
            fs::copy(&target.profile.path, &embedded)?;
        }

        let target_lookup: HashMap<PathBuf, BundleTarget> = signing_plan
            .targets
            .iter()
            .cloned()
            .map(|target| (target.path.clone(), target))
            .collect();

        let mut signing_order = signing_plan.targets.clone();
        signing_order.sort_by_key(|target| target.path.components().count());

        for target in signing_order
            .iter()
            .filter(|target| target.kind == BundleKind::AppExtension)
        {
            let name = normalize_bundle_component(&target.final_bundle_id);
            let entitlements =
                create_codesign_entitlements(target, extracted._temp_dir.path(), &name)?;
            sign_frameworks(&target.path, &identity)?;
            codesign_path(&target.path, &identity, Some(&entitlements))?;
        }

        let main_target = target_lookup.get(&extracted.app_path).ok_or_else(|| {
            TossError::Signing("main app target missing from signing plan".into())
        })?;
        let main_name = normalize_bundle_component(&main_target.final_bundle_id);
        let main_entitlements =
            create_codesign_entitlements(main_target, extracted._temp_dir.path(), &main_name)?;
        sign_frameworks(&extracted.app_path, &identity)?;
        codesign_path(&extracted.app_path, &identity, Some(&main_entitlements))?;
        verify_signed_app(&extracted.app_path)?;

        adapter.emit(WorkflowEvent::Installing {
            app_path: extracted.app_path.clone(),
            device_name: device_id.to_string(),
        })?;
        xcrun::install_app(device_id, &extracted.app_path)?;

        if launch {
            adapter.emit(WorkflowEvent::Launching {
                bundle_id: main_target.final_bundle_id.clone(),
                device_name: device_id.to_string(),
            })?;
            xcrun::launch_app(device_id, &main_target.final_bundle_id)?;
        }

        Ok(SignOutcome {
            app_path: extracted.app_path.clone(),
            final_bundle_id: main_target.final_bundle_id.clone(),
            launched: launch,
        })
    })();

    match cleanup_profiles(&cleanup_paths) {
        Ok(()) if !cleanup_paths.is_empty() => {
            adapter.emit(WorkflowEvent::CleanedTemporaryProfiles {
                count: cleanup_paths.len(),
            })?;
        }
        Ok(()) => {}
        Err(err) => adapter.emit(WorkflowEvent::Warning {
            message: format!("failed to clean temporary profiles: {}", err),
        })?,
    }

    result.map(|outcome| SignOutcome {
        app_path: app_path.clone(),
        final_bundle_id: if outcome.final_bundle_id.is_empty() {
            main_bundle_id
        } else {
            outcome.final_bundle_id
        },
        launched: outcome.launched,
    })
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
                team_id: Some("FRR2796948".into()),
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
    fn extension_bundle_id_follows_temp_main_bundle_id() {
        let config = Config {
            signing: SigningConfig {
                temp_bundle_prefix: Some("cn.yangym.tmp".into()),
                team_id: Some("FRR2796948".into()),
            },
            ..Config::default()
        };

        let derived = derive_extension_bundle_id(
            "com.example.app",
            "cn.yangym.tmp.app.1234",
            "com.example.app.widget",
            Path::new("/tmp/Widget.appex"),
            &config,
        )
        .unwrap();

        assert_eq!(derived, "cn.yangym.tmp.app.1234.widget");
    }

    #[test]
    fn created_profiles_only_include_new_exact_matches() {
        let before = vec![PathBuf::from("/tmp/existing.mobileprovision")];
        let after = vec![
            ProvisioningProfile {
                path: PathBuf::from("/tmp/existing.mobileprovision"),
                uuid: Some("old".into()),
                name: "existing".into(),
                team_ids: vec![],
                bundle_id_pattern: "cn.yangym.tmp.kazumi.1234".into(),
                expiration_epoch: None,
                provisioned_devices: vec![],
                provisions_all_devices: false,
            },
            ProvisioningProfile {
                path: PathBuf::from("/tmp/new.mobileprovision"),
                uuid: Some("new".into()),
                name: "new".into(),
                team_ids: vec![],
                bundle_id_pattern: "cn.yangym.tmp.kazumi.1234".into(),
                expiration_epoch: None,
                provisioned_devices: vec![],
                provisions_all_devices: false,
            },
            ProvisioningProfile {
                path: PathBuf::from("/tmp/wildcard.mobileprovision"),
                uuid: Some("wild".into()),
                name: "wildcard".into(),
                team_ids: vec![],
                bundle_id_pattern: "cn.yangym.tmp.*".into(),
                expiration_epoch: None,
                provisioned_devices: vec![],
                provisions_all_devices: false,
            },
        ];

        let created = profiles_created_for_bundle_id(&before, &after, "cn.yangym.tmp.kazumi.1234");
        assert_eq!(created, vec![PathBuf::from("/tmp/new.mobileprovision")]);
    }

    #[test]
    fn merged_entitlements_keep_original_capabilities_but_use_profile_values() {
        let profile = serde_json::json!({
            "application-identifier": "TEAMID.com.example.app",
            "com.apple.developer.team-identifier": "TEAMID",
            "keychain-access-groups": ["TEAMID.com.example.app"],
            "aps-environment": "development"
        });
        let original = serde_json::json!({
            "aps-environment": "production",
            "com.apple.developer.associated-domains": ["applinks:example.com"]
        });

        let merged = merged_entitlements(&profile, Some(&original));
        let merged_obj = merged.as_object().unwrap();

        assert_eq!(merged_obj.get("aps-environment").unwrap(), "development");
        assert!(merged_obj.contains_key("application-identifier"));
        assert!(!merged_obj.contains_key("com.apple.developer.associated-domains"));
    }
}
