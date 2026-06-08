//! Build-time configuration for `forage build`.
//!
//! A forage build runs inside the pinned base platform (the Arlen build-root,
//! forage-recipes.md section 13). Where that platform lives on disk is
//! deployment state, not part of any recipe, so it is read from
//! `~/.config/arlen/forage.toml` (the standard `~/.config/arlen/{component}.toml`
//! layout) or overridden by the `ARLEN_FORAGE_BASE_PLATFORM` /
//! `ARLEN_FORAGE_OUT_DIR` environment variables.
//!
//! There is deliberately **no built-in default base-platform path**: which
//! Debian snapshot is pinned and where it is installed is set by distro
//! provisioning, so an unconfigured build fails closed with a clear message
//! rather than guessing a location that does not exist.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// The `[build]` section of `~/.config/arlen/forage.toml`.
#[derive(Debug, Default, Deserialize)]
struct BuildSection {
    /// The pinned base-platform directory, mounted read-only as the build root.
    base_platform: Option<String>,
    /// Where produced `.lunpkg` files are written.
    out_dir: Option<String>,
}

/// The file shape of `~/.config/arlen/forage.toml`.
#[derive(Debug, Default, Deserialize)]
struct ForageToml {
    #[serde(default)]
    build: BuildSection,
}

/// Resolved build-time configuration.
#[derive(Debug, Clone)]
pub struct ForageBuildConfig {
    /// The configured base-platform directory, if any (env or file).
    base_platform: Option<PathBuf>,
    /// Where produced `.lunpkg` files are written.
    out_dir: PathBuf,
}

/// A failure resolving the build configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The config file exists but could not be read.
    #[error("could not read {path}: {source}")]
    Read {
        /// The config file path.
        path: PathBuf,
        /// The underlying io error.
        source: std::io::Error,
    },
    /// The config file is not valid TOML or has the wrong shape.
    #[error("invalid {path}: {source}")]
    Parse {
        /// The config file path.
        path: PathBuf,
        /// The TOML error.
        source: toml::de::Error,
    },
    /// No base platform is configured (neither env nor file).
    #[error(
        "no base platform configured; set [build].base_platform in ~/.config/arlen/forage.toml \
         or the ARLEN_FORAGE_BASE_PLATFORM environment variable"
    )]
    NoBasePlatform,
    /// A base platform is configured but the path is missing or not a directory.
    #[error("base platform {path} is not a directory")]
    BasePlatformMissing {
        /// The configured but unusable path.
        path: PathBuf,
    },
}

impl ForageBuildConfig {
    /// Load the configuration from `~/.config/arlen/forage.toml` (if present),
    /// then apply the `ARLEN_FORAGE_BASE_PLATFORM` / `ARLEN_FORAGE_OUT_DIR`
    /// environment overrides. A missing file is not an error (env or later
    /// fail-closed resolution covers it).
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = dirs::config_dir().map(|d| d.join("arlen/forage.toml"));
        Self::load_from(config_path.as_deref(), &EnvVars::from_process())
    }

    /// The resolution core, with the config path and environment injected so it
    /// can be tested without touching the real home directory or process env.
    fn load_from(config_path: Option<&Path>, env: &EnvVars) -> Result<Self, ConfigError> {
        let parsed = match config_path {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                })?;
                toml::from_str::<ForageToml>(&text).map_err(|source| ConfigError::Parse {
                    path: path.to_path_buf(),
                    source,
                })?
            }
            _ => ForageToml::default(),
        };

        // Environment overrides win over the file.
        let base_platform = env
            .base_platform
            .clone()
            .or(parsed.build.base_platform)
            .map(PathBuf::from);

        let out_dir = env
            .out_dir
            .clone()
            .or(parsed.build.out_dir)
            .map(PathBuf::from)
            .unwrap_or_else(default_out_dir);

        Ok(ForageBuildConfig {
            base_platform,
            out_dir,
        })
    }

    /// The directory produced `.lunpkg` files are written to.
    pub fn out_dir(&self) -> &Path {
        &self.out_dir
    }

    /// The validated base-platform directory: configured and present on disk.
    /// Fails closed when unset, or when the configured path is not a directory,
    /// so a build never silently proceeds against a missing build root.
    pub fn require_base_platform(&self) -> Result<&Path, ConfigError> {
        let path = self.base_platform.as_deref().ok_or(ConfigError::NoBasePlatform)?;
        if !path.is_dir() {
            return Err(ConfigError::BasePlatformMissing {
                path: path.to_path_buf(),
            });
        }
        Ok(path)
    }
}

/// The default `.lunpkg` output directory under the user cache.
fn default_out_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arlen/forage/packages")
}

/// The environment overrides, isolated so resolution is testable.
struct EnvVars {
    base_platform: Option<String>,
    out_dir: Option<String>,
}

impl EnvVars {
    /// Read the overrides from the process environment, treating an empty value
    /// as unset.
    fn from_process() -> Self {
        let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        EnvVars {
            base_platform: nonempty("ARLEN_FORAGE_BASE_PLATFORM"),
            out_dir: nonempty("ARLEN_FORAGE_OUT_DIR"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env() -> EnvVars {
        EnvVars {
            base_platform: None,
            out_dir: None,
        }
    }

    #[test]
    fn missing_file_yields_no_base_platform() {
        let cfg = ForageBuildConfig::load_from(None, &no_env()).unwrap();
        assert!(matches!(
            cfg.require_base_platform(),
            Err(ConfigError::NoBasePlatform)
        ));
    }

    #[test]
    fn file_base_platform_is_read_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        // The base platform exists as a directory, so it resolves.
        let platform = dir.path().join("platform");
        std::fs::create_dir(&platform).unwrap();
        let config = dir.path().join("forage.toml");
        std::fs::write(
            &config,
            format!("[build]\nbase_platform = \"{}\"\n", platform.display()),
        )
        .unwrap();
        let cfg = ForageBuildConfig::load_from(Some(&config), &no_env()).unwrap();
        assert_eq!(cfg.require_base_platform().unwrap(), platform);
    }

    #[test]
    fn configured_but_missing_path_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("forage.toml");
        std::fs::write(&config, "[build]\nbase_platform = \"/does/not/exist\"\n").unwrap();
        let cfg = ForageBuildConfig::load_from(Some(&config), &no_env()).unwrap();
        assert!(matches!(
            cfg.require_base_platform(),
            Err(ConfigError::BasePlatformMissing { .. })
        ));
    }

    #[test]
    fn env_overrides_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let platform = dir.path().join("env-platform");
        std::fs::create_dir(&platform).unwrap();
        let config = dir.path().join("forage.toml");
        std::fs::write(&config, "[build]\nbase_platform = \"/from/file\"\n").unwrap();
        let env = EnvVars {
            base_platform: Some(platform.display().to_string()),
            out_dir: None,
        };
        let cfg = ForageBuildConfig::load_from(Some(&config), &env).unwrap();
        assert_eq!(cfg.require_base_platform().unwrap(), platform);
    }

    #[test]
    fn out_dir_defaults_and_overrides() {
        let cfg = ForageBuildConfig::load_from(None, &no_env()).unwrap();
        assert!(cfg.out_dir().ends_with("arlen/forage/packages"));

        let env = EnvVars {
            base_platform: None,
            out_dir: Some("/tmp/pkgs".into()),
        };
        let cfg = ForageBuildConfig::load_from(None, &env).unwrap();
        assert_eq!(cfg.out_dir(), Path::new("/tmp/pkgs"));
    }

    #[test]
    fn malformed_toml_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("forage.toml");
        std::fs::write(&config, "[build]\nbase_platform = 42\n").unwrap();
        assert!(matches!(
            ForageBuildConfig::load_from(Some(&config), &no_env()),
            Err(ConfigError::Parse { .. })
        ));
    }
}
