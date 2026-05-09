//! v0.0.3-alpha.3: native coverage for the `Storage` trait contract
//! that the wasm host-storage shim implements.
//!
//! The actual `WasmHostStorage` lives in `motif-wasm` and bridges to a
//! JS object — exercising it end-to-end requires a wasm-bindgen-test
//! runner (browser / Node) which isn't in CI yet. To get coverage in
//! the meantime, this test stands up a `RustHostStorage` that
//! implements the same `Storage` contract via a `Vec<u8>` backing,
//! plus a [`StorageError::JsHostError`] return path. It exercises:
//!
//! - End-to-end commit through a host-supplied storage, matching the
//!   FileStorage / MemoryStorage acceptance shape.
//! - The `JsHostError` variant: the engine surfaces a host-thrown
//!   error from a storage method as a clean engine error.
//!
//! Note: the `MaybeSend` refactor lets `Storage` impls be `!Send` on
//! wasm32 only — native still requires `Send`. The wasm `!Send` path
//! is implicitly proven by `cargo build --target wasm32-unknown-
//! unknown -p motif-wasm` (which builds `WasmHostStorage` against the
//! same trait); a runtime test for it needs wasm-bindgen-test in CI
//! and is deferred to alpha.5+.
//!
//! When wasm-bindgen-test integration lands (alpha.5+), this file
//! gets a sibling that exercises the actual JS-bridged path.

use std::cell::RefCell;
use std::path::PathBuf;

use motif_core::storage::{Storage, StorageError};
use motif_core::{
    ControllerConfig, Engine, IdentityConfig, MotifConfig, Node, StorageConfig, Value,
};

/// Mirrors `motif_core::storage::HEADER_LEN` (pub(crate) inside
/// motif-core). Bump if the on-disk header layout grows.
const HEADER_LEN: u64 = 16;

fn config(path: PathBuf) -> MotifConfig {
    MotifConfig {
        identity: IdentityConfig {
            user_id: "u".into(),
            device_id: "d".into(),
        },
        controller: ControllerConfig {
            kind: "in-memory".into(),
        },
        storage: StorageConfig { path },
        capability: Default::default(),
        edge: Default::default(),
    }
}

/// Mirrors the v0.0.3-alpha.3 wasm host-storage contract:
/// in-process `Vec<u8>` backing, `RefCell` for interior mutability
/// (matching how the wasm shim holds JS state across the `&mut self`
/// Storage methods), and a `JsHostError` return path for the "throw
/// from JS" leg.
struct RustHostStorage {
    bytes: RefCell<Vec<u8>>,
    /// Set to `true` to make the next storage call return a
    /// `JsHostError`. Used by `engine_surfaces_host_storage_errors`.
    next_call_throws: RefCell<bool>,
}

impl RustHostStorage {
    fn new() -> Self {
        let mut bytes = Vec::with_capacity(HEADER_LEN as usize);
        // Same header layout as FileStorage / MemoryStorage so
        // recovery code stays backend-agnostic.
        bytes.extend_from_slice(b"MOTIF\0\0\x01");
        bytes.extend_from_slice(&3u32.to_le_bytes()); // current FORMAT_VERSION
        bytes.resize(HEADER_LEN as usize, 0);
        Self {
            bytes: RefCell::new(bytes),
            next_call_throws: RefCell::new(false),
        }
    }

    fn arm_throw(&self) {
        *self.next_call_throws.borrow_mut() = true;
    }
}

impl Storage for RustHostStorage {
    fn append(&mut self, bytes: &[u8]) -> Result<u64, StorageError> {
        if std::mem::replace(&mut *self.next_call_throws.borrow_mut(), false) {
            return Err(StorageError::JsHostError {
                message: "host append rejected".into(),
            });
        }
        let mut buf = self.bytes.borrow_mut();
        let offset = buf.len() as u64;
        buf.extend_from_slice(bytes);
        Ok(offset)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, StorageError> {
        let buf = self.bytes.borrow();
        let start = offset as usize;
        let end = start + len;
        if end > buf.len() {
            return Err(StorageError::JsHostError {
                message: format!("host read out of range: {start}..{end} (len={})", buf.len()),
            });
        }
        Ok(buf[start..end].to_vec())
    }

    fn len(&self) -> u64 {
        self.bytes.borrow().len() as u64
    }

    fn truncate(&mut self, new_len: u64) -> Result<(), StorageError> {
        if new_len < HEADER_LEN {
            return Err(StorageError::TruncateBelowHeader {
                new_len,
                header_len: HEADER_LEN,
            });
        }
        self.bytes.borrow_mut().truncate(new_len as usize);
        Ok(())
    }

    fn free_space(&self) -> Option<u64> {
        // Host-storage shims report what they know; this fake says
        // "1 GiB free" so the capability resolve path has something
        // to read.
        Some(1024 * 1024 * 1024)
    }
}

#[test]
fn engine_accepts_a_host_supplied_storage() {
    // Smoke test that Engine::open_with takes a host-shaped Storage
    // impl (not just FileStorage / MemoryStorage) and the capability
    // probe pulls free_space() through.
    let cfg = config(PathBuf::from(":host:"));
    let storage: Box<dyn Storage> = Box::new(RustHostStorage::new());
    let engine = Engine::open_with(&cfg, storage).expect("open with host storage");
    assert_eq!(engine.capability().storage_mb, Some(1024));
}

#[test]
fn host_storage_round_trips_through_engine() {
    let cfg = config(PathBuf::from(":host:"));
    let storage: Box<dyn Storage> = Box::new(RustHostStorage::new());
    let mut engine = Engine::open_with(&cfg, storage).expect("open");

    engine
        .insert_node(Node::new("a", "Person").with_property("idx", Value::I64(0)))
        .expect("insert a");
    engine
        .insert_node(Node::new("b", "Person").with_property("idx", Value::I64(1)))
        .expect("insert b");
    engine
        .insert_node(Node::new("c", "Person").with_property("idx", Value::I64(2)))
        .expect("insert c");

    // Round-trip: lookups go through the storage.read_at path.
    let a = engine.get_node("a").expect("get a").expect("a present");
    assert_eq!(a.label, "Person");
}

#[test]
fn engine_surfaces_host_storage_errors() {
    let cfg = config(PathBuf::from(":host:"));
    let storage = RustHostStorage::new();
    storage.arm_throw();
    let storage: Box<dyn Storage> = Box::new(storage);
    let mut engine = Engine::open_with(&cfg, storage).expect("open");

    // The next storage call (the append for this insert) throws
    // a JsHostError; the engine should surface it as an EngineError
    // wrapping that variant.
    let err = engine
        .insert_node(Node::new("oops", "Person"))
        .expect_err("insert should fail when host throws");
    let printed = format!("{err}");
    assert!(
        printed.contains("host append rejected") || printed.contains("host storage"),
        "expected host-storage error in message, got: {printed}"
    );
}
