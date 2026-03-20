use std::path::Path;

/// Loads and validates a TOML configuration file for a component.
/// Implemented by the real filesystem loader and a mock for tests.
pub trait ConfigLoader: Send + Sync {
    type Config: serde::de::DeserializeOwned;

    fn load(path: &Path) -> Result<Self::Config, ConfigError>;
}

#[derive(Debug)]
pub enum ConfigError {
    NotFound(String),
    ParseError(String),
    ValidationError(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotFound(path) => write!(f, "config not found: {path}"),
            ConfigError::ParseError(msg) => write!(f, "parse error: {msg}"),
            ConfigError::ValidationError(msg) => write!(f, "validation error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}
