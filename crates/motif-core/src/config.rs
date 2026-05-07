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
/// constructed in code for tests).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MotifConfig {
    pub identity: IdentityConfig,
    pub controller: ControllerConfig,
    pub storage: StorageConfig,
    /// Deterministic facts about the host hardware (RAM MB, CPU cores,
    /// arch, etc.). Motif reports facts; controller decides policy.
    /// All fields default to `None` / sensible blanks so existing
    /// configs from earlier alphas keep parsing.
    #[serde(default)]
    pub capability: CapabilityConfig,
    /// Edge-strategy knobs (foreshadow eagerness, retention, schema
    /// cache, controller retry policy). Defaults preserve the v0.0.2
    /// behavior in effect before this section was introduced.
    #[serde(default)]
    pub edge: EdgeConfig,
}

/// Per-user + per-device identity. Both fields are mandatory: a compromised
/// device must still be distinguishable from the same user on a different
/// device, and the controller relies on the pair for audit and conflict
/// resolution. Motif compares opaque tokens; the host owns the auth flow
/// (MOTIF.md decision 4).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct IdentityConfig {
    pub user_id: String,
    pub device_id: String,
}

/// Controller routing hint. v0.0.2-alpha.2 dropped the strongly-typed
/// `ControllerKind` enum: motif-core is controller-agnostic
/// (MOTIF.md decision 18) so a finite enum here would have to enumerate
/// every bridge, which violates the OSS posture. `kind` is now an
/// opaque string that the host's bridge crate (or `motif-cli` /
/// `motif-wasm`) interprets.
///
/// Conventional values:
/// - `"in-memory"` — the default `InMemoryController` shipped in
///   motif-core; useful for tests, local development, and the alpha.2
///   threading-model validation.
/// - `"external"` — host wires its own [`crate::sync::Controller`]
///   programmatically; motif-core does not auto-instantiate one.
/// - `"<bridge-name>"` — interpreted by the corresponding bridge crate
///   (e.g. `motif-surreal-bridge` looks for `kind = "surreal"`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ControllerConfig {
    pub kind: String,
}

/// Where Motif's local single-file store lives.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageConfig {
    pub path: PathBuf,
}

/// Deterministic facts about the host hardware. Motif reports these
/// to the controller (via the bridge) so the controller can choose
/// between `edge-is-tiny` (cache + foreshadow) and `edge-is-free`
/// (local execution) routing strategies. Per MOTIF.md decision 20,
/// no qualitative labels — numbers and well-defined enums only.
///
/// Auto-discovery is v0.0.3+; for now hosts populate manually at
/// install time. All fields are optional; absent fields just mean
/// the host has not declared them.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CapabilityConfig {
    pub ram_mb: Option<u64>,
    pub cpu_cores: Option<u32>,
    pub storage_mb: Option<u64>,
    pub arch: Option<String>,
    pub gpu_present: Option<bool>,
}

/// Edge strategy knobs. v0.0.2-alpha.4 wires the
/// `controller_retry_*` fields into the worker's retry / backoff
/// state machine; the others are parsed and stored on the engine
/// for future alphas (foreshadow buffer-mode, retention compaction,
/// schema-fetch policy) but do not yet change behavior.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EdgeConfig {
    /// `true`  — apply mutations to the local store immediately,
    ///           marked foreshadow=true (`edge-is-free`).
    /// `false` — buffer mutations until controller confirms; reads
    ///           return server state only (`edge-is-tiny`).
    /// v0.0.2-alpha.4 parses but does not yet enforce `false`;
    /// behaviour is always `true`. Buffer-mode lands in alpha.5.
    #[serde(default = "default_true")]
    pub foreshadow_eager: bool,

    /// Seconds to keep mutation-log entries after the controller
    /// confirms them. `0` evicts immediately. Parsed in alpha.4 but
    /// log compaction lands in alpha.5+.
    #[serde(default = "default_retention_secs")]
    pub retention_confirmed_secs: u64,

    /// `"push"` — controller pushes schema; motif caches the latest.
    /// `"fetch"` — motif fetches schema lazily on first reference.
    /// Only `"push"` is implemented in v0.0.2.
    #[serde(default = "default_schema_cache")]
    pub schema_cache: String,

    /// Maximum exponential-backoff delay (ms) between retries when
    /// `Controller::apply` returns `ControllerError::Transient`.
    /// Backoff doubles from 100ms up to this cap. Default: 30s.
    #[serde(default = "default_retry_max_backoff_ms")]
    pub controller_retry_max_backoff_ms: u64,

    /// Maximum number of retry attempts before the worker drops a
    /// mutation. `0` means unlimited (default). Permanent errors
    /// short-circuit regardless.
    #[serde(default)]
    pub controller_retry_max_attempts: u32,
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            foreshadow_eager: true,
            retention_confirmed_secs: default_retention_secs(),
            schema_cache: default_schema_cache(),
            controller_retry_max_backoff_ms: default_retry_max_backoff_ms(),
            controller_retry_max_attempts: 0,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_retention_secs() -> u64 {
    3600
}
fn default_schema_cache() -> String {
    "push".to_string()
}
fn default_retry_max_backoff_ms() -> u64 {
    30_000
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

    /// Re-emit the config as TOML. Used for the `--print-config`
    /// round-trip exit criterion.
    pub fn to_toml_string(&self) -> String {
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
        assert_eq!(cfg.controller.kind, "in-memory");
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
    fn parses_arbitrary_kind() {
        // motif-core itself doesn't validate `kind`; that's the bridge's
        // job. Confirm any non-empty string parses.
        let src = r#"
            [identity]
            user_id = "u"
            device_id = "d"
            [controller]
            kind = "future-bridge-not-yet-implemented"
            [storage]
            path = "./x.db"
        "#;
        let cfg = MotifConfig::from_toml_str(src).unwrap();
        assert_eq!(cfg.controller.kind, "future-bridge-not-yet-implemented");
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
