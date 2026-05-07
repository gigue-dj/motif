//! Controller worker scaffolding.
//!
//! v0.0.2-alpha.4 grew this from the simple loop-and-apply of alpha.2
//! into a proper state machine: the worker first calls
//! `Controller::connect(capability)`, then loops applying mutations
//! with exponential-backoff retry on `ControllerError::Transient`.
//! `Permanent` errors short-circuit (the mutation stays foreshadow=true
//! on disk; alpha.5's `replay_unconfirmed` flow can re-feed it later).
//!
//! Retry policy comes from `EdgeConfig.controller_retry_*`. Backoff
//! starts at 100ms and doubles up to `controller_retry_max_backoff_ms`.
//! `controller_retry_max_attempts = 0` (the default) means unlimited
//! retries.
//!
//! On worker shutdown the controller's `flush()` is called once after
//! the channel returns EOF. v0.0.2-alpha.4 considers shutdown
//! best-effort (drop the engine to drop the channel sender to signal
//! EOF; native joins lazily, wasm just lets the task complete).

use super::{Controller, ControllerError, Mutation, MutationLog};
use crate::config::{CapabilityConfig, EdgeConfig};

/// Spawn a worker that drains `log` into `controller`. Returns a
/// `WorkerHandle` whose drop semantics are loose for v0.0.2-alpha.4 —
/// the worker stops when its channel sender is dropped (which happens
/// when the engine drops). Tests that need deterministic shutdown
/// should drop the engine before reading the controller handle.
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
    spawn_impl(log, controller, capability, edge_config)
}

#[allow(dead_code)]
pub struct WorkerHandle {
    _private: (),
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_impl<C: Controller>(
    log: &MutationLog,
    controller: C,
    capability: CapabilityConfig,
    edge_config: EdgeConfig,
) -> WorkerHandle {
    use std::sync::mpsc;
    use std::thread;

    let (tx, rx) = mpsc::channel::<Mutation>();

    log.set_forwarder(Box::new(move |m| {
        let _ = tx.send(m);
    }));

    let mut controller = controller;
    thread::spawn(move || {
        // Best-effort connect. A Transient/Permanent here means we
        // skip the apply loop and go straight to flush — a real bridge
        // can decide whether retries-of-connect are worth doing in
        // alpha.5+. For v0.0.2-alpha.4 we keep it simple: connect
        // failure → no apply, just flush.
        if let Err(e) = controller.connect(&capability) {
            eprintln!("motif: controller.connect failed: {e}");
            controller.flush();
            return;
        }

        while let Ok(m) = rx.recv() {
            retry_apply_native(&mut controller, &m, &edge_config);
        }
        controller.flush();
    });

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
                eprintln!(
                    "motif: dropping mutation seq={} (permanent): {reason}",
                    m.local_seq
                );
                return;
            }
            Err(ControllerError::Transient { reason: _ }) => {
                attempt = attempt.saturating_add(1);
                if edge.controller_retry_max_attempts > 0
                    && attempt >= edge.controller_retry_max_attempts
                {
                    eprintln!(
                        "motif: dropping mutation seq={} after {attempt} retries",
                        m.local_seq
                    );
                    return;
                }
                std::thread::sleep(Duration::from_millis(backoff_ms));
                backoff_ms =
                    (backoff_ms.saturating_mul(2)).min(edge.controller_retry_max_backoff_ms);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn_impl<C: Controller>(
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
        if let Err(_) = controller.connect(&capability) {
            controller.flush();
            return;
        }

        while let Some(m) = rx.next().await {
            retry_apply_wasm(&mut controller, &m, &edge_config).await;
        }
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
            Err(ControllerError::Permanent { .. }) => return,
            Err(ControllerError::Transient { .. }) => {
                attempt = attempt.saturating_add(1);
                if edge.controller_retry_max_attempts > 0
                    && attempt >= edge.controller_retry_max_attempts
                {
                    return;
                }
                wasm_sleep(backoff_ms).await;
                backoff_ms =
                    (backoff_ms.saturating_mul(2)).min(edge.controller_retry_max_backoff_ms);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn wasm_sleep(_ms: u64) {
    // v0.0.2-alpha.4 has no sleep primitive on wasm without pulling
    // a heavier dep (gloo-timers / web-sys::setTimeout). For the
    // alpha-4 milestone the wasm worker will spin through the
    // microtask queue without sleeping; v0.0.3+ adds proper backoff
    // when a real bridge needs it. Documented in LIMITATIONS.md.
    futures_util::future::ready(()).await;
}
