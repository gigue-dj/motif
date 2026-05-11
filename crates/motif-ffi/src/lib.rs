//! Motif C ABI shim — v0.0.3-alpha.4.
//!
//! Tiny `extern "C"` surface for hosts that consume motif as a
//! `cdylib`: primarily Swift on iOS (via `static let` C-string imports
//! or a SwiftPM `cSettings` umbrella) and Kotlin on Android (via JNI).
//! `motif-wasm` covers the wasm runtimes; `motif-ffi` covers everything
//! that isn't a wasm host.
//!
//! ## Scope
//!
//! v0.0.3-alpha.4 ships the build pipeline and a single symbol
//! ([`motif_version`]) — enough to prove the cdylib link path works
//! end-to-end and that motif-core compiles cleanly for
//! `aarch64-apple-ios` + `aarch64-linux-android`. The engine surface
//! (`motif_open` / `motif_query` / `motif_close`) lands in v0.0.4
//! alongside the first crates.io publish, when the public API is
//! stable enough to commit to a C ABI shape.
//!
//! ## Why a separate crate vs. cdylib on motif-core directly?
//!
//! `motif-core` stays `unsafe_code = "forbid"`. A C ABI surface needs
//! `unsafe extern "C"` for anything that takes a pointer. Carving the
//! FFI seam into its own crate keeps the engine itself pure Rust;
//! anyone packaging Motif for an FFI consumer pulls in `motif-ffi`
//! explicitly. This mirrors the bridges-architecture posture: the
//! engine doesn't know about its consumers.
//!
//! ## ABI stability
//!
//! No promises through v0.0.x. Hosts pinning to specific motif tags
//! is fine; expecting the C ABI to survive a minor bump isn't. The
//! v0.1.0 milestone freezes the API surface (per `MOTIF.md` long-run
//! strategy).

// motif-ffi is the dedicated FFI seam: by construction it needs
// `unsafe` for any signature that takes pointers (and for invoking
// `CStr::from_ptr` / similar in tests). The workspace-level
// `unsafe_code = "forbid"` lint stays in place for every other
// crate; this is the one place it's explicitly relaxed.
#![allow(unsafe_code)]

use std::os::raw::c_char;

/// Returns a pointer to a NUL-terminated C string with motif's
/// package version (e.g. `"0.0.2"`). The returned pointer lives in
/// the cdylib's static memory — do not free, do not retain across
/// dylib unload.
///
/// Used by hosts as a smoke check that the FFI link path works.
/// In Swift:
///
/// ```swift
/// import Foundation
/// let cstr = motif_version()!  // UnsafePointer<CChar>
/// let v = String(cString: cstr)
/// // v == "0.0.2"
/// ```
#[no_mangle]
pub extern "C" fn motif_version() -> *const c_char {
    // `concat!` builds a `&'static str`; appending "\0" gives us a
    // NUL-terminated byte sequence in static memory. Casting `*const u8`
    // → `*const c_char` is safe (both are 1-byte aligned).
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn motif_version_is_nul_terminated_and_matches_pkg_version() {
        // SAFETY: `motif_version` returns a pointer to static memory
        // ending in NUL — see the function's invariants.
        let cstr = unsafe { CStr::from_ptr(motif_version()) };
        let s = cstr.to_str().expect("UTF-8");
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
    }
}
