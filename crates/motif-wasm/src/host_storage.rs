//! Host-supplied storage shim — v0.0.3-alpha.3.
//!
//! `wasm32-unknown-unknown` has no filesystem. Through v0.0.3-alpha.2
//! the wasm path was forced to `MemoryStorage` (working set only;
//! mutations evaporated when the wasm runtime tore down). This module
//! plugs that gap by giving the host a JS interface to implement
//! against whatever it has access to: OPFS in browsers, the iOS app
//! sandbox, the Android internal-storage directory, or anything else.
//!
//! ## TypeScript-shaped contract
//!
//! ```typescript
//! interface MotifHostStorage {
//!   /// Append `bytes` to the store. Return the byte offset where the
//!   /// write began. Implementations MUST persist the write before
//!   /// returning (e.g. fsync / OPFS commit) — motif's recovery
//!   /// semantics assume durability.
//!   append(bytes: Uint8Array): number;
//!
//!   /// Read `len` bytes starting at `offset`. Returns the bytes;
//!   /// throwing on out-of-range read is fine — motif surfaces it
//!   /// as a `StorageError::JsHostError`.
//!   readAt(offset: number, len: number): Uint8Array;
//!
//!   /// Total byte length of the store, including the 16-byte header.
//!   len(): number;
//!
//!   /// Truncate the store to `newLen` bytes. Implementations MUST
//!   /// persist the truncation before returning.
//!   truncate(newLen: number): void;
//!
//!   /// Bytes the host knows are still available on the underlying
//!   /// medium. Optional — return `undefined` if the host can't
//!   /// answer (motif treats `undefined` as "unknown" and skips
//!   /// `[capability].storage_mb` accordingly).
//!   freeSpace?(): number | undefined;
//! }
//! ```
//!
//! ## Why `number` and not `bigint`?
//!
//! wasm-bindgen marshals `u64` as JavaScript `BigInt`, which is
//! unergonomic for hosts and non-trivial in OPFS shims. `number` is
//! exact up to 2^53 (~9 PB) — well past the size of any motif store
//! we can reasonably foresee, and it matches how the JS storage APIs
//! (`OPFS.size()`, etc.) report bytes.
//!
//! ## Why no OPFS reference impl?
//!
//! Per the v0.0.3 plan, OPFS / app-sandbox / RN bridges are host
//! territory. Shipping one canonical reference impl would couple
//! motif-wasm to a host platform; the trait + interface keeps motif
//! controller-agnostic the same way the `Controller` trait keeps it
//! controller-transport-agnostic.

use motif_core::{Storage, StorageError};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// Host-implemented storage interface. The host constructs a JS
    /// object matching the typescript contract above and passes it to
    /// `Motif::open_with_host_storage`. Motif holds the object for
    /// the lifetime of the engine and calls into it on every
    /// append / read / truncate.
    #[wasm_bindgen(typescript_type = "MotifHostStorage")]
    pub type MotifHostStorage;

    #[wasm_bindgen(method, structural, catch)]
    fn append(this: &MotifHostStorage, bytes: &[u8]) -> Result<f64, JsValue>;

    #[wasm_bindgen(method, structural, catch, js_name = readAt)]
    fn read_at(
        this: &MotifHostStorage,
        offset: f64,
        len: u32,
    ) -> Result<js_sys::Uint8Array, JsValue>;

    #[wasm_bindgen(method, structural)]
    fn len(this: &MotifHostStorage) -> f64;

    #[wasm_bindgen(method, structural, catch)]
    fn truncate(this: &MotifHostStorage, new_len: f64) -> Result<(), JsValue>;

    #[wasm_bindgen(method, structural, js_name = freeSpace)]
    fn free_space(this: &MotifHostStorage) -> JsValue;
}

/// Rust wrapper around a host-supplied [`MotifHostStorage`] JS object.
/// Implements [`Storage`] so the engine can use it interchangeably
/// with `FileStorage` / `MemoryStorage`.
///
/// `WasmHostStorage` is `!Send` because it holds a `JsValue`. That's
/// fine — wasm32-unknown-unknown is single-threaded by default, and
/// the [`Storage`] trait drops its `Send` bound on wasm via the
/// [`motif_core::storage::MaybeSend`] marker.
pub struct WasmHostStorage {
    inner: MotifHostStorage,
}

impl WasmHostStorage {
    pub fn new(inner: MotifHostStorage) -> Self {
        Self { inner }
    }
}

impl Storage for WasmHostStorage {
    fn append(&mut self, bytes: &[u8]) -> Result<u64, StorageError> {
        let offset = self.inner.append(bytes).map_err(js_to_storage_err)?;
        Ok(offset as u64)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, StorageError> {
        let arr = self
            .inner
            .read_at(offset as f64, len as u32)
            .map_err(js_to_storage_err)?;
        Ok(arr.to_vec())
    }

    fn len(&self) -> u64 {
        self.inner.len() as u64
    }

    fn truncate(&mut self, new_len: u64) -> Result<(), StorageError> {
        // Defensive: the trait guarantees new_len >= HEADER_LEN
        // because Storage::truncate's contract requires it, and
        // FileStorage / MemoryStorage already enforce that. Mirror the
        // guard here so a host shim doesn't have to re-check it
        // (and so the host can't accidentally clobber the magic).
        const HEADER_LEN: u64 = 16;
        if new_len < HEADER_LEN {
            return Err(StorageError::TruncateBelowHeader {
                new_len,
                header_len: HEADER_LEN,
            });
        }
        self.inner
            .truncate(new_len as f64)
            .map_err(js_to_storage_err)?;
        Ok(())
    }

    fn free_space(&self) -> Option<u64> {
        let v = self.inner.free_space();
        // The host returns `number | undefined`. Treat anything
        // non-numeric (undefined / null / a string thrown back) as
        // "unknown" — `[capability].storage_mb` then stays `None`
        // and the controller decides what to do.
        v.as_f64().map(|n| n as u64)
    }
}

fn js_to_storage_err(e: JsValue) -> StorageError {
    let message = e
        .as_string()
        .or_else(|| {
            // Some hosts throw `Error` objects; pull `.message` if
            // present. Best-effort.
            js_sys::Reflect::get(&e, &JsValue::from_str("message"))
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| "<no message>".to_string());
    StorageError::JsHostError { message }
}
