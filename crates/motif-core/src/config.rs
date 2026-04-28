//! TOML-backed configuration for Motif. The host application owns the
//! `motif.toml` file; Motif treats it as read-only at open time. See
//! `motif.toml.example` at the repo root for the canonical schema.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse TOML: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Top-level Motif configuration. Loaded from a `motif.toml` file (or
/// constructed in code for tests). All fields are required in v0.0.1 — we
/// will introduce defaults only when we have a concrete reason to.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MotifConfig {
    pub identity: IdentityConfig,
    pub controller: ControllerConfig,
    pub storage: StorageConfig,
}

/// Per-user + per-device identity. Both fields are mandatory: a compromised
/// device must still be distinguishable from the same user on a different
/// device, and the controller relies on the pair for audit and conflict
/// resolution. Real auth tokens are out of scope for v0.0.1; these are
/// opaque strings that the controller will validate later.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct IdentityConfig {
    pub user_id: String,
    pub device_id: String,
}

/// Where Motif sends committed mutations. v0.0.1 only supports the
/// `in-memory` controller — real SurrealDB transport lands in v0.0.2.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ControllerConfig {
    pub kind: ControllerKind,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ControllerKind {
    InMemory,
}

/// Where Motif's local single-file store lives. The actual storage engine
/// lands in alpha.3; for alpha.2 this is just a validated path.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageConfig {
    pub path: PathBuf,
}

impl MotifConfig {
    pub fn from_toml_str(s: &str) -> Result<Self, ConfigError> {
        toml::from_str(s).map_err(ConfigError::from)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let body = std::fs::read_to_string(path).map_err(|e| ConfigError::Read {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::from_toml_str(&body)
    }

    /// Re-emit the config as TOML. Used for the `--print-config` round-trip
    /// exit criterion.
    pub fn to_toml_string(&self) -> String {
        // toml::to_string only fails on serializer bugs, not on our types.
        toml::to_string(self).expect("MotifConfig is always serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [identity]
        user_id = "u_abc"
        device_id = "d_xyz"

        [controller]
        kind = "in-memory"

        [storage]
        path = "./motif.db"
    "#;

    #[test]
    fn parses_sample() {
        let cfg = MotifConfig::from_toml_str(SAMPLE).unwrap();
        assert_eq!(cfg.identity.user_id, "u_abc");
        assert_eq!(cfg.identity.device_id, "d_xyz");
        assert_eq!(cfg.controller.kind, ControllerKind::InMemory);
        assert_eq!(cfg.storage.path, PathBuf::from("./motif.db"));
    }

    #[test]
    fn round_trips() {
        let cfg = MotifConfig::from_toml_str(SAMPLE).unwrap();
        let emitted = cfg.to_toml_string();
        let reparsed = MotifConfig::from_toml_str(&emitted).unwrap();
        assert_eq!(cfg, reparsed);
    }

    #[test]
    fn missing_field_is_an_error() {
        let bad = r#"
            [identity]
            user_id = "u_abc"

            [controller]
            kind = "in-memory"

            [storage]
            path = "./motif.db"
        "#;
        let err = MotifConfig::from_toml_str(bad).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }
}
