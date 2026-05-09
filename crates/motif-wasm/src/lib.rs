//! `motif-wasm` exposes `motif-core` to host languages via
//! `wasm32-unknown-unknown` + `wasm-bindgen`. v0.0.2-alpha.2 wires an
//! in-memory controller behind `Engine::with_controller`; on wasm the
//! worker drains via `wasm-bindgen-futures::spawn_local` so the engine
//! commit path stays low-latency.
//!
//! - [`Motif::open`] takes the same TOML config as the native API,
//!   constructs an in-memory engine, wires an `InMemoryController` via
//!   the worker so `controller_applied_count` is observable from JS.
//! - [`Motif::open_with_host_storage`] (v0.0.3-alpha.3) takes the same
//!   TOML config plus a host-supplied JS object implementing the
//!   [`MotifHostStorage`] interface (OPFS, app sandbox, RN bridge,
//!   etc.). Closes the wasm-`MemoryStorage`-only gap. See
//!   [`host_storage`] for the TypeScript-shaped contract and the
//!   "no canonical OPFS impl" rationale.
//! - [`Motif::query`] takes a Cypher string and a JSON-encoded params
//!   object, returns a JSON-encoded `QueryResult`.
//!
//! Errors are surfaced as `JsError`. The host sees them as plain JS
//! `Error` instances with a string message.

mod host_storage;

use std::collections::BTreeMap;

use motif_core::{Engine, InMemoryController, InMemoryHandle, MotifConfig, Params, Value};
use wasm_bindgen::prelude::*;

pub use host_storage::{MotifHostStorage, WasmHostStorage};

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
    controller: InMemoryHandle,
}

#[wasm_bindgen]
impl Motif {
    /// Open an in-memory Motif instance configured by `toml_src`.
    /// `storage.path` from the config is ignored — for persistence
    /// across page loads / app restarts on wasm, use
    /// [`Motif::open_with_host_storage`] with an OPFS / app-sandbox /
    /// RN-bridge backed JS object.
    #[wasm_bindgen(constructor)]
    pub fn open(toml_src: &str) -> Result<Motif, JsError> {
        let cfg = MotifConfig::from_toml_str(toml_src).map_err(to_js)?;
        let controller = InMemoryController::new();
        let handle = controller.handle();
        let engine = Engine::open_in_memory(&cfg)
            .map_err(to_js)?
            .with_controller(controller);
        Ok(Motif {
            engine,
            controller: handle,
        })
    }

    /// Open Motif with a host-supplied [`MotifHostStorage`] backend.
    /// The host implements the JS interface (`append` / `readAt` /
    /// `len` / `truncate` / optional `freeSpace`) against whatever
    /// persistent medium it has access to. v0.0.3-alpha.3 closes the
    /// wasm-`MemoryStorage`-only gap; concrete reference impls (OPFS,
    /// iOS app sandbox, etc.) are host territory.
    pub fn open_with_host_storage(
        toml_src: &str,
        storage: MotifHostStorage,
    ) -> Result<Motif, JsError> {
        let cfg = MotifConfig::from_toml_str(toml_src).map_err(to_js)?;
        let host_storage = Box::new(WasmHostStorage::new(storage));
        let controller = InMemoryController::new();
        let handle = controller.handle();
        let engine = Engine::open_with(&cfg, host_storage)
            .map_err(to_js)?
            .with_controller(controller);
        Ok(Motif {
            engine,
            controller: handle,
        })
    }

    /// Run a query. `params_json` must be a JSON object mapping string
    /// keys to scalar values (`null`, `bool`, integers, floats, strings).
    /// Returns the `QueryResult` as a JSON string.
    pub fn query(&mut self, cypher: &str, params_json: &str) -> Result<String, JsError> {
        let params = parse_params(params_json)?;
        let result = self.engine.query(cypher, &params).map_err(to_js)?;
        serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Diagnostic: how many mutations the wasm controller worker has
    /// applied so far. The wasm worker drains via
    /// `wasm-bindgen-futures::spawn_local`; for hosts that need to
    /// observe progress without yielding control, this count lags the
    /// engine's commit count by however many mutations the microtask
    /// queue has not yet drained. Yield to JS (`await Promise.resolve()`
    /// or similar) before sampling for an up-to-date count.
    pub fn controller_applied_count(&self) -> usize {
        self.controller.len()
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
