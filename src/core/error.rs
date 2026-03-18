#[derive(Debug, thiserror::Error)]
pub enum TossError {
    #[error("config error: {0}")]
    Config(String),

    #[error("device error: {0}")]
    Device(String),

    #[error("project error: {0}")]
    Project(String),

    #[error("xcrun failed: {0}")]
    Xcrun(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml parse error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),

    #[error("toml serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("{0}")]
    UserCancelled(String),
}

pub type Result<T> = std::result::Result<T, TossError>;
