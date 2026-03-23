use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::config::Config;
use super::error::{Result, TossError};
use super::sign;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum CleanCategory {
    Config,
    TempProfiles,
    ProvisioningProfiles,
    DerivedData,
    CargoTarget,
}

impl CleanCategory {
    pub fn key(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::TempProfiles => "temp-profiles",
            Self::ProvisioningProfiles => "provisioning-profiles",
            Self::DerivedData => "derived-data",
            Self::CargoTarget => "cargo-target",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Config => "Config",
            Self::TempProfiles => "Temporary profiles",
            Self::ProvisioningProfiles => "Provisioning profiles",
            Self::DerivedData => "DerivedData",
            Self::CargoTarget => "Cargo target",
        }
    }

    pub fn owner(self) -> &'static str {
        match self {
            Self::Config | Self::TempProfiles => "toss",
            Self::ProvisioningProfiles | Self::DerivedData => "xcode",
            Self::CargoTarget => "rust",
        }
    }

    pub fn safety(self) -> &'static str {
        match self {
            Self::Config => "caution",
            Self::TempProfiles => "safe",
            Self::ProvisioningProfiles | Self::DerivedData | Self::CargoTarget => "external",
        }
    }

    pub fn purpose(self) -> &'static str {
        match self {
            Self::Config => "Stored defaults, aliases, projects, and signing settings.",
            Self::TempProfiles => {
                "Temporary provisioning profiles created for toss signing fallback."
            }
            Self::ProvisioningProfiles => "All provisioning profiles shared with Xcode signing.",
            Self::DerivedData => "Xcode build intermediates and build cache.",
            Self::CargoTarget => "Rust build artifacts for this repo.",
        }
    }

    pub fn supports_delete(self) -> bool {
        !matches!(self, Self::Config)
    }

    pub fn is_safe_default(self) -> bool {
        matches!(self, Self::TempProfiles)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "config" => Some(Self::Config),
            "temp-profiles" => Some(Self::TempProfiles),
            "provisioning-profiles" => Some(Self::ProvisioningProfiles),
            "derived-data" => Some(Self::DerivedData),
            "cargo-target" => Some(Self::CargoTarget),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Config,
            Self::TempProfiles,
            Self::ProvisioningProfiles,
            Self::DerivedData,
            Self::CargoTarget,
        ]
    }
}

#[derive(Debug, Clone)]
pub struct CleanItem {
    pub category: CleanCategory,
    pub paths: Vec<PathBuf>,
    pub size_bytes: u64,
    pub path_count: usize,
    pub deletable: bool,
}

#[derive(Debug)]
pub struct CleanReport {
    pub items: Vec<CleanItem>,
    pub notes: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeleteSummary {
    pub deleted_paths: usize,
    pub reclaimed_bytes: u64,
}

pub fn collect_report(config: &Config, cwd: &Path) -> Result<CleanReport> {
    let mut items = Vec::new();

    if let Some(item) = collect_config_item()? {
        items.push(item);
    }

    if let Some(item) = collect_temp_profiles_item(config)? {
        items.push(item);
    }

    if let Some(item) = collect_all_profiles_item()? {
        items.push(item);
    }

    if let Some(item) = collect_derived_data_item()? {
        items.push(item);
    }

    if let Some(item) = collect_cargo_target_item(cwd)? {
        items.push(item);
    }

    let notes = vec![
        "Runtime temp files from IPA extraction, devicectl JSON output, and entitlement conversion use tempfile-managed locations and normally disappear automatically.".into(),
        "Items owned by Xcode or Rust are included for visibility, but toss does not own them exclusively.".into(),
    ];

    Ok(CleanReport { items, notes })
}

pub fn parse_delete_categories(values: &[String], all_safe: bool) -> Result<Vec<CleanCategory>> {
    let mut selected = BTreeSet::new();

    for value in values {
        let Some(category) = CleanCategory::parse(value) else {
            let choices = CleanCategory::all()
                .iter()
                .map(|category| category.key())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(TossError::Config(format!(
                "unknown clean category '{}' — valid values: {}",
                value, choices
            )));
        };
        selected.insert(category);
    }

    if all_safe {
        for category in CleanCategory::all() {
            if category.is_safe_default() {
                selected.insert(*category);
            }
        }
    }

    Ok(selected.into_iter().collect())
}

pub fn delete_categories(
    report: &CleanReport,
    categories: &[CleanCategory],
) -> Result<DeleteSummary> {
    let mut summary = DeleteSummary::default();

    for category in categories {
        let item = report
            .items
            .iter()
            .find(|item| item.category == *category)
            .cloned();
        let Some(item) = item else {
            continue;
        };

        if !item.deletable {
            return Err(TossError::Config(format!(
                "category '{}' cannot be deleted automatically",
                category.key()
            )));
        }

        for path in &item.paths {
            if !path.exists() {
                continue;
            }
            delete_path(path)?;
            summary.deleted_paths += 1;
        }
        summary.reclaimed_bytes += item.size_bytes;
    }

    Ok(summary)
}

pub fn legacy_temp_profile_cleanup(config: &Config) -> Result<DeleteSummary> {
    let cwd = std::env::current_dir()?;
    let report = collect_report(config, &cwd)?;
    delete_categories(&report, &[CleanCategory::TempProfiles])
}

fn collect_config_item() -> Result<Option<CleanItem>> {
    let path = Config::path()?;
    if !path.exists() {
        return Ok(None);
    }
    let size_bytes = compute_path_size(&path)?;
    Ok(Some(CleanItem {
        category: CleanCategory::Config,
        paths: vec![path],
        size_bytes,
        path_count: 1,
        deletable: false,
    }))
}

fn collect_temp_profiles_item(config: &Config) -> Result<Option<CleanItem>> {
    let Some(prefix) = config.signing.temp_bundle_prefix.as_deref() else {
        return Ok(None);
    };

    let mut paths = Vec::new();
    for inspection in sign::inspect_provisioning_profiles()? {
        if let Some(profile) = inspection.profile
            && profile.bundle_id_pattern.starts_with(prefix)
        {
            paths.push(profile.path);
        }
    }

    build_file_item(CleanCategory::TempProfiles, paths)
}

fn collect_all_profiles_item() -> Result<Option<CleanItem>> {
    let paths = provisioning_profile_files()?;
    build_file_item(CleanCategory::ProvisioningProfiles, paths)
}

fn collect_derived_data_item() -> Result<Option<CleanItem>> {
    let Some(home) = dirs::home_dir() else {
        return Ok(None);
    };
    let path = home.join("Library/Developer/Xcode/DerivedData");
    build_root_item(CleanCategory::DerivedData, path)
}

fn collect_cargo_target_item(cwd: &Path) -> Result<Option<CleanItem>> {
    build_root_item(CleanCategory::CargoTarget, cwd.join("target"))
}

fn build_root_item(category: CleanCategory, path: PathBuf) -> Result<Option<CleanItem>> {
    if !path.exists() {
        return Ok(None);
    }

    let size_bytes = compute_path_size(&path)?;
    let path_count = count_nodes(&path)?;
    Ok(Some(CleanItem {
        category,
        paths: vec![path],
        size_bytes,
        path_count,
        deletable: category.supports_delete(),
    }))
}

fn build_file_item(category: CleanCategory, paths: Vec<PathBuf>) -> Result<Option<CleanItem>> {
    if paths.is_empty() {
        return Ok(None);
    }

    let mut size_bytes = 0;
    for path in &paths {
        size_bytes += compute_path_size(path)?;
    }

    Ok(Some(CleanItem {
        category,
        path_count: paths.len(),
        paths,
        size_bytes,
        deletable: category.supports_delete(),
    }))
}

fn provisioning_profile_files() -> Result<Vec<PathBuf>> {
    let Some(home) = dirs::home_dir() else {
        return Ok(Vec::new());
    };

    let dirs = [
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ];

    let mut files = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(dir)?.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "mobileprovision") {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn delete_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn compute_path_size(path: &Path) -> Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        total += compute_path_size(&entry.path())?;
    }
    Ok(total)
}

fn count_nodes(path: &Path) -> Result<usize> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() {
        return Ok(1);
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    let mut total = 1;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        total += count_nodes(&entry.path())?;
    }
    Ok(total)
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{} B", bytes);
    }

    let mut value = bytes as f64;
    let mut unit_index = 0usize;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }
    format!("{:.1} {}", value, UNITS[unit_index])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn format_bytes_uses_human_units() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
    }

    #[test]
    fn parse_delete_categories_adds_safe_defaults() {
        let categories = parse_delete_categories(&[], true).unwrap();
        assert_eq!(categories, vec![CleanCategory::TempProfiles]);
    }

    #[test]
    fn parse_delete_categories_rejects_unknown_values() {
        let err = parse_delete_categories(&[String::from("unknown")], false).unwrap_err();
        assert!(err.to_string().contains("unknown clean category"));
    }

    #[test]
    fn compute_path_size_aggregates_nested_files() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.path().join("a.txt"), b"abcd").unwrap();
        fs::write(nested.join("b.txt"), b"123456").unwrap();

        let size = compute_path_size(dir.path()).unwrap();
        assert_eq!(size, 10);
    }

    #[test]
    fn delete_categories_removes_selected_paths() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("temp.mobileprovision");
        let mut handle = fs::File::create(&file).unwrap();
        writeln!(handle, "data").unwrap();

        let report = CleanReport {
            items: vec![CleanItem {
                category: CleanCategory::TempProfiles,
                paths: vec![file.clone()],
                size_bytes: 5,
                path_count: 1,
                deletable: true,
            }],
            notes: Vec::new(),
        };

        let summary = delete_categories(&report, &[CleanCategory::TempProfiles]).unwrap();
        assert!(!file.exists());
        assert_eq!(summary.deleted_paths, 1);
        assert_eq!(summary.reclaimed_bytes, 5);
    }

    #[test]
    fn delete_categories_rejects_non_deletable_category() {
        let report = CleanReport {
            items: vec![CleanItem {
                category: CleanCategory::Config,
                paths: vec![PathBuf::from("/tmp/config.toml")],
                size_bytes: 1,
                path_count: 1,
                deletable: false,
            }],
            notes: Vec::new(),
        };

        let err = delete_categories(&report, &[CleanCategory::Config]).unwrap_err();
        assert!(err.to_string().contains("cannot be deleted automatically"));
    }
}
