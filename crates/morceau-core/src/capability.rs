//! Capability probe — v0.0.3-alpha.1.
//!
//! Per MORCEAU.md decision 20, morceau reports **deterministic facts** about
//! the host: numbers and well-defined enums, never qualitative labels
//! ("medium", "sufficient"). The host or controller decides what counts
//! as enough.
//!
//! v0.0.2 stored a host-supplied [`CapabilityConfig`] from `morceau.toml`
//! and forwarded it to the controller. v0.0.3-alpha.1 makes the probe
//! the **primary** mechanism (because the host's view of resources isn't
//! always morceau's view — they may not be colocated or allocated the
//! same budget) and treats `morceau.toml`'s `[capability]` as a
//! field-level **override**:
//!
//! - If the host declared `cpu_cores = 4` in TOML, that wins.
//! - If a field is absent, morceau probes — `std::thread::available_parallelism`
//!   for cores, `sysinfo::System::total_memory` for RAM,
//!   [`crate::storage::Storage::free_space`] for disk, `cfg!(target_arch)`
//!   for arch.
//!
//! Probes that can't answer return `None` rather than guessing — bridges
//! making policy decisions need to know the difference between "small"
//! and "unknown".
//!
//! On `wasm32-unknown-unknown`, native probes (sysinfo / fs2) aren't
//! available; v0.0.3-alpha.3 wires the storage shim and adds
//! `web-sys`-driven probes (`navigator.hardwareConcurrency`,
//! `navigator.deviceMemory`, `navigator.storage.estimate()`). Until
//! then, wasm probes return `None` for every dynamic field; arch is
//! always `"wasm32"` via `std::env::consts::ARCH`.

use crate::config::CapabilityConfig;
use crate::storage::Storage;

/// Probe the host's deterministic capability facts.
///
/// `storage` is consulted for `storage_mb` via [`Storage::free_space`].
/// Native targets fill `cpu_cores` / `ram_mb` from `available_parallelism`
/// / `sysinfo`; wasm32 uses `navigator.hardwareConcurrency` and
/// `navigator.deviceMemory` (Chrome-only for the latter; returns
/// `None` on Firefox / Safari).
pub fn probe(storage: &dyn Storage) -> CapabilityConfig {
    CapabilityConfig {
        ram_mb: probe_ram_mb(),
        cpu_cores: probe_cpu_cores(),
        storage_mb: probe_storage_mb(storage),
        arch: Some(std::env::consts::ARCH.to_string()),
        // No portable signal for GPU presence; the host overrides
        // via TOML if it knows.
        gpu_present: None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn probe_ram_mb() -> Option<u64> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    // sysinfo reports total_memory in bytes (since 0.30); convert to MB.
    let total_bytes = sys.total_memory();
    if total_bytes == 0 {
        None
    } else {
        Some(total_bytes / (1024 * 1024))
    }
}

#[cfg(target_arch = "wasm32")]
fn probe_ram_mb() -> Option<u64> {
    // navigator.deviceMemory is reported in GiB and intentionally
    // coarse (rounded to one of 0.25, 0.5, 1, 2, 4, 8 per the spec
    // — anti-fingerprinting). Chrome-only; Firefox / Safari return
    // undefined and we fall through to None.
    let gib = navigator_property("deviceMemory")?.as_f64()?;
    Some((gib * 1024.0) as u64)
}

#[cfg(not(target_arch = "wasm32"))]
fn probe_cpu_cores() -> Option<u32> {
    std::thread::available_parallelism()
        .ok()
        .map(|n| n.get() as u32)
}

#[cfg(target_arch = "wasm32")]
fn probe_cpu_cores() -> Option<u32> {
    navigator_property("hardwareConcurrency")?
        .as_f64()
        .map(|n| n as u32)
}

#[cfg(target_arch = "wasm32")]
fn navigator_property(name: &str) -> Option<wasm_bindgen::JsValue> {
    use wasm_bindgen::JsValue;
    // `Reflect::get` on a missing-or-non-object target returns Err
    // (the JS-side `TypeError`), which `.ok()?` swallows. Callers
    // chain `.as_f64()?` / `.as_string()?` to filter undefined / null /
    // wrong-type values — no need to guard those here.
    let global = js_sys::global();
    let nav = js_sys::Reflect::get(&global, &JsValue::from_str("navigator")).ok()?;
    js_sys::Reflect::get(&nav, &JsValue::from_str(name)).ok()
}

fn probe_storage_mb(storage: &dyn Storage) -> Option<u64> {
    storage.free_space().map(|bytes| bytes / (1024 * 1024))
}

/// Resolve a host-declared [`CapabilityConfig`] against a probe result.
/// Per-field: TOML declaration wins where present; probe fills the rest.
///
/// This is the value [`Engine::capability`](crate::Engine::capability)
/// returns post-alpha.1 — the merged truth, not the raw declaration.
pub fn resolve(declared: &CapabilityConfig, probed: CapabilityConfig) -> CapabilityConfig {
    CapabilityConfig {
        ram_mb: declared.ram_mb.or(probed.ram_mb),
        cpu_cores: declared.cpu_cores.or(probed.cpu_cores),
        storage_mb: declared.storage_mb.or(probed.storage_mb),
        arch: declared.arch.clone().or(probed.arch),
        gpu_present: declared.gpu_present.or(probed.gpu_present),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    #[test]
    fn probe_fills_arch_always() {
        let storage = MemoryStorage::new();
        let probed = probe(&storage);
        assert_eq!(probed.arch.as_deref(), Some(std::env::consts::ARCH));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn probe_fills_cpu_cores_on_native() {
        let storage = MemoryStorage::new();
        let probed = probe(&storage);
        assert!(
            probed.cpu_cores.is_some(),
            "available_parallelism should answer on native"
        );
        // Sanity bound — avoid flake by not asserting an exact number.
        assert!(probed.cpu_cores.unwrap() >= 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn probe_fills_ram_mb_on_native() {
        let storage = MemoryStorage::new();
        let probed = probe(&storage);
        assert!(
            probed.ram_mb.is_some(),
            "sysinfo should report total_memory on native"
        );
        // CI runners have at least 1 GiB; pin a generous floor to
        // catch a sysinfo regression without flaking on tiny VMs.
        assert!(probed.ram_mb.unwrap() >= 256);
    }

    #[test]
    fn probe_storage_mb_is_none_for_memory_backend() {
        let storage = MemoryStorage::new();
        let probed = probe(&storage);
        assert_eq!(probed.storage_mb, None);
    }

    #[test]
    fn probe_gpu_present_is_none_no_portable_signal() {
        let storage = MemoryStorage::new();
        let probed = probe(&storage);
        assert_eq!(probed.gpu_present, None);
    }

    #[test]
    fn resolve_per_field_override_declared_wins() {
        let declared = CapabilityConfig {
            ram_mb: Some(2048),
            cpu_cores: None,
            storage_mb: None,
            arch: None,
            gpu_present: Some(true),
        };
        let probed = CapabilityConfig {
            ram_mb: Some(8192),
            cpu_cores: Some(8),
            storage_mb: Some(100_000),
            arch: Some("aarch64".to_string()),
            gpu_present: Some(false),
        };
        let resolved = resolve(&declared, probed);
        // Declared fields win.
        assert_eq!(resolved.ram_mb, Some(2048));
        assert_eq!(resolved.gpu_present, Some(true));
        // Probe fills the rest.
        assert_eq!(resolved.cpu_cores, Some(8));
        assert_eq!(resolved.storage_mb, Some(100_000));
        assert_eq!(resolved.arch.as_deref(), Some("aarch64"));
    }

    #[test]
    fn resolve_empty_declared_is_pure_probe() {
        let declared = CapabilityConfig::default();
        let probed = CapabilityConfig {
            ram_mb: Some(4096),
            cpu_cores: Some(4),
            storage_mb: Some(50_000),
            arch: Some("x86_64".to_string()),
            gpu_present: None,
        };
        let resolved = resolve(&declared, probed.clone());
        assert_eq!(resolved, probed);
    }

    #[test]
    fn resolve_none_in_both_stays_none() {
        let declared = CapabilityConfig::default();
        let probed = CapabilityConfig::default();
        let resolved = resolve(&declared, probed);
        assert_eq!(resolved, CapabilityConfig::default());
    }
}
