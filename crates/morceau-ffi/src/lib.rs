//! Morceau C ABI shim.
//!
//! Tiny `extern "C"` surface for hosts that consume morceau as a
//! `cdylib`: primarily Swift on iOS (via `static let` C-string imports
//! or a SwiftPM `cSettings` umbrella) and Kotlin on Android (via JNI).
//! `morceau-wasm` covers the wasm runtimes; `morceau-ffi` covers everything
//! that isn't a wasm host.
//!
//! ## Scope
//!
//! v0.0.4 ships the build pipeline and a single symbol
//! ([`morceau_version`]) — enough to prove the cdylib link path works
//! end-to-end and that morceau-core compiles cleanly for
//! `aarch64-apple-ios` + `aarch64-linux-android`. The engine surface
//! (`morceau_open` / `morceau_query` / `morceau_close`) lands in v0.0.5+
//! when the public API is stable enough to commit to a C ABI shape;
//! the audit checklist below stays in place to gate that expansion.
//!
//! ## Why a separate crate vs. cdylib on morceau-core directly?
//!
//! `morceau-core` stays `unsafe_code = "forbid"`. A C ABI surface needs
//! `unsafe extern "C"` for anything that takes a pointer. Carving the
//! FFI seam into its own crate keeps the engine itself pure Rust;
//! anyone packaging Morceau for an FFI consumer pulls in `morceau-ffi`
//! explicitly. This mirrors the bridges-architecture posture: the
//! engine doesn't know about its consumers.
//!
//! ## ABI stability
//!
//! No promises through v0.0.x. Hosts pinning to specific morceau tags
//! is fine; expecting the C ABI to survive a minor bump isn't. The
//! v0.1.0 milestone freezes the API surface (per `MORCEAU.md` long-run
//! strategy).
//!
//! ## Unsafe audit
//!
//! `morceau-ffi` is the workspace's **only** `unsafe_code = "forbid"`
//! relaxer. The audit checklist gates every new `unsafe` block before
//! a crates.io publish:
//!
//! - **SAFETY comment** above the block naming the caller invariants
//!   it relies on.
//! - **Pointer parameters validated** (non-null + alignment) before
//!   any deref. Errors return a documented error code rather than UB.
//! - **Handle ownership / lifetime documented** in the function's
//!   rustdoc, and exercised by at least one test (round-trip:
//!   `morceau_open` → `morceau_query` → `morceau_close` when those land).
//! - **No `unsafe impl Send/Sync`** without an audited justification
//!   that names the synchronization primitive enforcing the bound.
//! - **C ABI types are stable**: no Rust enum discriminants leak
//!   where the host can't see the layout (use opaque pointers /
//!   `#[repr(C)]` with explicit discriminants).
//!
//! ### v0.0.4-alpha.5 audit pass (BEFORE first crates.io publish)
//!
//! Surface audited: one symbol (`morceau_version`), one `unsafe` block
//! (a unit test calling `CStr::from_ptr`). Results:
//!
//! - `morceau_version` body uses no `unsafe`. The `concat!(...).as_ptr()
//!   as *const c_char` chain is a primitive-pointer cast (1-byte
//!   alignment in both directions), not a deref. ✓
//! - The test's `unsafe { CStr::from_ptr(morceau_version()) }` carries
//!   a SAFETY comment naming the static-memory + NUL-termination
//!   invariants. ✓
//! - No pointer parameters in the v0.0.4 surface — pointer-validation
//!   checklist item is N/A. ✓
//! - No `unsafe impl Send/Sync` anywhere in the crate. ✓
//! - C ABI surface is `*const c_char` only — no Rust enums leak. ✓
//! - Workspace `unsafe_code = "forbid"` lint verified intact on
//!   morceau-core / morceau-wasm / morceau-cli; morceau-ffi is the only
//!   relaxer. ✓
//!
//! The v0.0.5+ engine surface (`morceau_open` / `morceau_query` /
//! `morceau_close`) re-runs this audit against the expanded surface
//! before the next crates.io publish.

// morceau-ffi is the dedicated FFI seam: by construction it needs
// `unsafe` for any signature that takes pointers (and for invoking
// `CStr::from_ptr` / similar in tests). The workspace-level
// `unsafe_code = "forbid"` lint stays in place for every other
// crate; this is the one place it's explicitly relaxed.
#![allow(unsafe_code)]

use std::os::raw::c_char;

/// Returns a pointer to a NUL-terminated C string with morceau's
/// package version (e.g. `"0.0.2"`). The returned pointer lives in
/// the cdylib's static memory — do not free, do not retain across
/// dylib unload.
///
/// Used by hosts as a smoke check that the FFI link path works.
/// In Swift:
///
/// ```swift
/// import Foundation
/// let cstr = morceau_version()!  // UnsafePointer<CChar>
/// let v = String(cString: cstr)
/// // v == "0.0.2"
/// ```
#[no_mangle]
pub extern "C" fn morceau_version() -> *const c_char {
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
    fn morceau_version_is_nul_terminated_and_matches_pkg_version() {
        // SAFETY: `morceau_version` returns a pointer to static memory
        // ending in NUL — see the function's invariants.
        let cstr = unsafe { CStr::from_ptr(morceau_version()) };
        let s = cstr.to_str().expect("UTF-8");
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
    }
}
