//! `motif-wasm` exposes `motif-core` to host languages via
//! `wasm32-unknown-unknown` + `wasm-bindgen`. v0.0.1-alpha.2 is a stub:
//! we publish a `version()` and a TOML config validator so the toolchain
//! is exercised end-to-end. Real bindings (open, query, mutation hooks)
//! land in alpha.5 once the engine exists.

use motif_core::config::{ConfigError, MotifConfig};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Validate a TOML configuration string. Returns `Ok(())` if the config
/// parses, or a string describing the parse error. Used by the alpha.2
/// build-pipeline smoke test.
#[wasm_bindgen]
pub fn validate_config(toml_src: &str) -> Result<(), JsError> {
    MotifConfig::from_toml_str(toml_src)
        .map(|_| ())
        .map_err(|e: ConfigError| JsError::new(&e.to_string()))
}
