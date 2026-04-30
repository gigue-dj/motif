//! `motif-wasm` exposes `motif-core` to host languages via
//! `wasm32-unknown-unknown` + `wasm-bindgen`. v0.0.1-alpha.5 ships the
//! minimum surface needed by a Swift or Rust host to open a database
//! and run queries:
//!
//! - [`Motif::open`] takes the same TOML config as the native API,
//!   constructs an in-memory engine (the wasm32 target has no
//!   filesystem; a host-provided storage shim is post-v0.0.1), and
//!   wires an in-memory `MutationLog` so `mutation_count` is observable.
//! - [`Motif::query`] takes a Cypher string and a JSON-encoded params
//!   object, returns a JSON-encoded `QueryResult`.
//!
//! Errors are surfaced as `JsError`. The host sees them as plain JS
//! `Error` instances with a string message.

use std::collections::BTreeMap;
use std::sync::Arc;

use motif_core::{Engine, MotifConfig, MutationLog, Params, Value};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Validate a TOML configuration string. Returns `Ok(())` if the config
/// parses, or a string describing the parse error. Carried over from
/// alpha.2 so existing host smoke tests still work.
#[wasm_bindgen]
pub fn validate_config(toml_src: &str) -> Result<(), JsError> {
    MotifConfig::from_toml_str(toml_src)
        .map(|_| ())
        .map_err(|e| JsError::new(&e.to_string()))
}

#[wasm_bindgen]
pub struct Motif {
    engine: Engine,
    log: Arc<MutationLog>,
}

#[wasm_bindgen]
impl Motif {
    /// Open an in-memory Motif instance configured by `toml_src`.
    /// `storage.path` from the config is currently ignored on wasm —
    /// see crate docs.
    #[wasm_bindgen(constructor)]
    pub fn open(toml_src: &str) -> Result<Motif, JsError> {
        let cfg = MotifConfig::from_toml_str(toml_src).map_err(to_js)?;
        let log = Arc::new(MutationLog::new());
        let engine = Engine::open_in_memory(&cfg)
            .map_err(to_js)?
            .with_mutation_log(log.clone());
        Ok(Motif { engine, log })
    }

    /// Run a query. `params_json` must be a JSON object mapping string
    /// keys to scalar values (`null`, `bool`, integers, floats, strings).
    /// Returns the `QueryResult` as a JSON string.
    pub fn query(&mut self, cypher: &str, params_json: &str) -> Result<String, JsError> {
        let params = parse_params(params_json)?;
        let result = self.engine.query(cypher, &params).map_err(to_js)?;
        serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Diagnostic: how many mutations have been queued for the
    /// controller. Used by the alpha.5 architectural-validation test on
    /// the host side.
    pub fn mutation_count(&self) -> usize {
        self.log.buffered_len()
    }
}

fn parse_params(json: &str) -> Result<Params, JsError> {
    if json.trim().is_empty() {
        return Ok(Params::new());
    }
    let raw: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| JsError::new(&format!("params must be a JSON object: {e}")))?;
    let map = raw
        .as_object()
        .ok_or_else(|| JsError::new("params must be a JSON object"))?;

    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    for (k, v) in map {
        out.insert(k.clone(), json_value_to_motif_value(v)?);
    }
    Ok(out)
}

fn json_value_to_motif_value(v: &serde_json::Value) -> Result<Value, JsError> {
    use serde_json::Value as J;
    Ok(match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::I64(i)
            } else if let Some(f) = n.as_f64() {
                Value::F64(f)
            } else {
                return Err(JsError::new("number out of range"));
            }
        }
        J::String(s) => Value::String(s.clone()),
        J::Array(_) | J::Object(_) => {
            return Err(JsError::new(
                "params currently support scalar values only (null/bool/int/float/string)",
            ));
        }
    })
}

fn to_js<E: std::fmt::Display>(e: E) -> JsError {
    JsError::new(&e.to_string())
}
