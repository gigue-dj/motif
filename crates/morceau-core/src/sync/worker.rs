//! Controller worker scaffolding.
//!
//! v0.0.2-alpha.4 grew this from the simple loop-and-apply of alpha.2
//! into a proper state machine: the worker first calls
//! `Controller::connect(capability)`, then loops applying mutations
//! with exponential-backoff retry on `ControllerError::Transient`.
//! `Permanent` errors short-circuit (the mutation stays foreshadow=true
//! on disk; alpha.5's `replay_unconfirmed` flow can re-feed it later).
//!
//! v0.0.3-alpha.2 routes the native task spawn through a [`Spawner`]
//! trait so hosts on iOS / Android can swap in GCD / coroutines
//! instead of `std::thread::spawn`. Default is [`StdThreadSpawner`].
//! The wasm path still uses `wasm_bindgen_futures::spawn_local`
//! directly — the host's wasm runtime *is* the spawner. wasm sleep
//! also moves from a `future::ready` no-op to a real `gloo-timers`
//! `setTimeout`, so retry backoff actually backs off.
//!
//! Retry policy comes from `EdgeConfig.controller_retry_*`. Backoff
//! starts at 100ms and doubles up to `controller_retry_max_backoff_ms`.
//! `controller_retry_max_attempts = 0` (the default) means unlimited
//! retries.
//!
//! On worker shutdown the controller's `flush()` is called once after
//! the channel returns EOF. Shutdown is best-effort: drop the engine
//! to drop the channel sender, and native joins lazily / wasm just
//! lets the task complete.

use super::{Controller, ControllerError, Mutation, MutationLog};
use crate::config::{CapabilityConfig, EdgeConfig};

#[cfg(not(target_arch = "wasm32"))]
use super::{Spawner, StdThreadSpawner};

/// Spawn a worker that drains `log` into `controller` using the
/// default [`StdThreadSpawner`] on native (or `spawn_local` on wasm).
/// Returns a `WorkerHandle` whose drop semantics are loose — the
/// worker stops when its channel sender is dropped (which happens
/// when the engine drops). Tests that need deterministic shutdown
/// should drop the engine before reading the controller handle.
///
/// Hosts that want a non-default spawner on native call
/// [`spawn_controller_worker_with`] instead. The wasm path always uses
/// `spawn_local`; the trait is native-only in v0.0.3-alpha.2.
///
/// `capability` and `edge_config` are passed by value because the
/// worker owns its copy for the lifetime of the task. The engine
/// keeps its own copy for `Engine::capability()` / `Engine::edge()`
/// readouts.
pub fn spawn_controller_worker<C: Controller>(
    log: &MutationLog,
    controller: C,
    capability: CapabilityConfig,
    edge_config: EdgeConfig,
) -> WorkerHandle {
    #[cfg(not(target_arch = "wasm32"))]
    {
        spawn_with_spawner(log, controller, capability, edge_config, StdThreadSpawner)
    }
    #[cfg(target_arch = "wasm32")]
    {
        spawn_impl_wasm(log, controller, capability, edge_config)
    }
}

/// Native-only: spawn the worker through a host-supplied [`Spawner`].
/// On wasm there is no equivalent — `spawn_local` is the only seam
/// and overriding it would mean replacing the microtask queue, which
/// isn't a real use case. Hosts on iOS / Android (post-alpha.4
/// cdylib targets) wire their preferred runtime here.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_controller_worker_with<C: Controller, S: Spawner>(
    log: &MutationLog,
    controller: C,
    capability: CapabilityConfig,
    edge_config: EdgeConfig,
    spawner: S,
) -> WorkerHandle {
    spawn_with_spawner(log, controller, capability, edge_config, spawner)
}

#[allow(dead_code)]
pub struct WorkerHandle {
    _private: (),
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_with_spawner<C: Controller, S: Spawner>(
    log: &MutationLog,
    controller: C,
    capability: CapabilityConfig,
    edge_config: EdgeConfig,
    spawner: S,
) -> WorkerHandle {
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<Mutation>();

    log.set_forwarder(Box::new(move |m| {
        let _ = tx.send(m);
    }));

    let mut controller = controller;
    spawner.spawn_worker(Box::new(move || {
        let _span = tracing::info_span!("controller_worker").entered();
        // Best-effort connect. A Transient/Permanent here means we
        // skip the apply loop and go straight to flush — a real bridge
        // can decide whether retries-of-connect are worth doing in
        // alpha.5+. For v0.0.2-alpha.4 we keep it simple: connect
        // failure → no apply, just flush.
        if let Err(e) = controller.connect(&capability) {
            tracing::warn!(error = %e, "controller.connect failed; skipping apply loop");
            controller.flush();
            return;
        }
        tracing::debug!("controller connected; entering apply loop");

        while let Ok(m) = rx.recv() {
            retry_apply_native(&mut controller, &m, &edge_config);
        }
        tracing::debug!("controller worker draining; calling flush()");
        controller.flush();
    }));

    WorkerHandle { _private: () }
}

#[cfg(not(target_arch = "wasm32"))]
fn retry_apply_native<C: Controller>(controller: &mut C, m: &Mutation, edge: &EdgeConfig) {
    use std::time::Duration;

    let mut attempt: u32 = 0;
    let mut backoff_ms: u64 = 100;

    loop {
        match controller.apply(m) {
            Ok(()) => return,
            Err(ControllerError::Permanent { reason }) => {
                tracing::warn!(
                    seq = m.local_seq,
                    reason = %reason,
                    "dropping mutation (permanent error)"
                );
                return;
            }
            Err(ControllerError::Transient { reason }) => {
                attempt = attempt.saturating_add(1);
                if edge.controller_retry_max_attempts > 0
                    && attempt >= edge.controller_retry_max_attempts
                {
                    tracing::warn!(
                        seq = m.local_seq,
                        attempts = attempt,
                        "dropping mutation after retry budget exhausted"
                    );
                    return;
                }
                tracing::debug!(
                    seq = m.local_seq,
                    attempt,
                    backoff_ms,
                    reason = %reason,
                    "transient apply error; sleeping before retry"
                );
                std::thread::sleep(Duration::from_millis(backoff_ms));
                backoff_ms =
                    (backoff_ms.saturating_mul(2)).min(edge.controller_retry_max_backoff_ms);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn_impl_wasm<C: Controller>(
    log: &MutationLog,
    controller: C,
    capability: CapabilityConfig,
    edge_config: EdgeConfig,
) -> WorkerHandle {
    use futures_channel::mpsc;
    use futures_util::stream::StreamExt;

    let (tx, mut rx) = mpsc::unbounded::<Mutation>();

    log.set_forwarder(Box::new(move |m| {
        let _ = tx.unbounded_send(m);
    }));

    let mut controller = controller;
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = controller.connect(&capability) {
            tracing::warn!(error = %e, "controller.connect failed; skipping apply loop");
            controller.flush();
            return;
        }
        tracing::debug!("controller connected; entering apply loop");

        while let Some(m) = rx.next().await {
            retry_apply_wasm(&mut controller, &m, &edge_config).await;
        }
        tracing::debug!("controller worker draining; calling flush()");
        controller.flush();
    });

    WorkerHandle { _private: () }
}

#[cfg(target_arch = "wasm32")]
async fn retry_apply_wasm<C: Controller>(controller: &mut C, m: &Mutation, edge: &EdgeConfig) {
    let mut attempt: u32 = 0;
    let mut backoff_ms: u64 = 100;

    loop {
        match controller.apply(m) {
            Ok(()) => return,
            Err(ControllerError::Permanent { reason }) => {
                tracing::warn!(
                    seq = m.local_seq,
                    reason = %reason,
                    "dropping mutation (permanent error)"
                );
                return;
            }
            Err(ControllerError::Transient { reason }) => {
                attempt = attempt.saturating_add(1);
                if edge.controller_retry_max_attempts > 0
                    && attempt >= edge.controller_retry_max_attempts
                {
                    tracing::warn!(
                        seq = m.local_seq,
                        attempts = attempt,
                        "dropping mutation after retry budget exhausted"
                    );
                    return;
                }
                tracing::debug!(
                    seq = m.local_seq,
                    attempt,
                    backoff_ms,
                    reason = %reason,
                    "transient apply error; awaiting wasm_sleep before retry"
                );
                wasm_sleep(backoff_ms).await;
                backoff_ms =
                    (backoff_ms.saturating_mul(2)).min(edge.controller_retry_max_backoff_ms);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn wasm_sleep(ms: u64) {
    // v0.0.3-alpha.2: real backoff via gloo-timers' setTimeout-backed
    // future. The cap on `ms` matters because gloo-timers takes a
    // u32; in practice `EdgeConfig.controller_retry_max_backoff_ms`
    // defaults to 30_000 which is well within u32, so the saturating
    // cast is defensive against a host configuring something
    // pathological.
    let ms_u32 = u32::try_from(ms).unwrap_or(u32::MAX);
    gloo_timers::future::TimeoutFuture::new(ms_u32).await;
}
