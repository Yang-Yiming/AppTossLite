use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::error::{Result, TossError};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub devices: DevicesConfig,
    #[serde(default)]
    pub signing: SigningConfig,
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectConfig>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DefaultsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DevicesConfig {
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SigningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_bundle_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    #[default]
    Xcode,
    Ipa,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub kind: ProjectKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub build_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipa_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tossed_at: Option<String>,
}

impl ProjectConfig {
    pub fn is_ipa(&self) -> bool {
        self.kind == ProjectKind::Ipa
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| TossError::Config("cannot determine config directory".into()))?;
        Ok(config_dir.join("toss").join("config.toml"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Config::default());
        }
        let content = fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_project_without_last_tossed_at() {
        let config: Config = toml::from_str(
            r#"
            [projects.demo]
            build_dir = "/tmp/Demo"
            "#,
        )
        .unwrap();

        assert_eq!(config.projects["demo"].build_dir, "/tmp/Demo");
        assert_eq!(config.projects["demo"].last_tossed_at, None);
    }

    #[test]
    fn serializes_last_tossed_at_when_present() {
        let mut config = Config::default();
        config.projects.insert(
            "demo".into(),
            ProjectConfig {
                kind: ProjectKind::Xcode,
                path: None,
                build_dir: "/tmp/Demo".into(),
                bundle_id: None,
                app_name: None,
                ipa_path: None,
                original_name: None,
                last_tossed_at: Some("2026-03-25T12:34:56Z".into()),
            },
        );

        let serialized = toml::to_string(&config).unwrap();

        assert!(serialized.contains("last_tossed_at = \"2026-03-25T12:34:56Z\""));
    }

    #[test]
    fn loads_legacy_project_as_xcode() {
        let config: Config = toml::from_str(
            r#"
            [projects.demo]
            build_dir = "/tmp/Demo"
            "#,
        )
        .unwrap();

        assert_eq!(config.projects["demo"].kind, ProjectKind::Xcode);
    }

    #[test]
    fn serializes_ipa_project_fields() {
        let mut config = Config::default();
        config.projects.insert(
            "demo".into(),
            ProjectConfig {
                kind: ProjectKind::Ipa,
                path: None,
                build_dir: String::new(),
                bundle_id: Some("com.example.demo".into()),
                app_name: None,
                ipa_path: Some("/tmp/demo.ipa".into()),
                original_name: Some("Demo.ipa".into()),
                last_tossed_at: None,
            },
        );

        let serialized = toml::to_string(&config).unwrap();

        assert!(serialized.contains("kind = \"ipa\""));
        assert!(serialized.contains("ipa_path = \"/tmp/demo.ipa\""));
    }
}
