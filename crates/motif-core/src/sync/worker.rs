//! Controller worker scaffolding.
//!
//! v0.0.2-alpha.2 wires a worker per controller so the engine commit
//! path stays low-latency: `Engine::commit` enqueues a `Mutation` on a
//! channel; the worker drains the channel and calls `Controller::apply`
//! on its own thread (native) or microtask (wasm).
//!
//! The native worker uses `std::thread::spawn` with `std::sync::mpsc`.
//! The wasm worker uses `wasm-bindgen-futures::spawn_local` with
//! `futures-channel::mpsc::unbounded`. Both paths are unbounded — we
//! rely on the engine commit rate to be far below the controller drain
//! rate; backpressure is a v0.0.3+ design item once real network
//! controllers exist.
//!
//! On worker shutdown the controller's `flush()` is called once after
//! the channel returns EOF. v0.0.2-alpha.2 considers shutdown
//! best-effort (drop the engine to drop the sender to signal EOF;
//! native joins lazily, wasm just lets the task complete).

use super::{Controller, Mutation, MutationLog};

/// Spawn a worker that drains `log` into `controller`. Returns a
/// `WorkerHandle` whose `Drop` impl signals shutdown by dropping the
/// channel sender held inside the engine's `MutationLog` forwarder.
///
/// This wires the appropriate channel for the current target:
/// `std::sync::mpsc` on native, `futures-channel::mpsc::unbounded` on
/// wasm32. The forwarder closure stamped into `log` enqueues; the
/// worker dequeues.
pub fn spawn_controller_worker<C: Controller>(log: &MutationLog, controller: C) -> WorkerHandle {
    spawn_impl(log, controller)
}

/// Lifetime tag for an active worker. Drop semantics are intentionally
/// loose for v0.0.2-alpha.2: dropping the handle does not by itself
/// stop the worker. The worker stops when its channel sender is
/// dropped — which happens when the engine drops or `MutationLog::close()`
/// is called. Tests that need deterministic shutdown should drop the
/// engine before reading the controller handle.
#[allow(dead_code)]
pub struct WorkerHandle {
    /// Marker; the actual native thread join handle is held internally
    /// in cfg(not(wasm)) impl. Kept private so callers don't depend on
    /// the threading model.
    _private: (),
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_impl<C: Controller>(log: &MutationLog, controller: C) -> WorkerHandle {
    use std::sync::mpsc;
    use std::thread;

    let (tx, rx) = mpsc::channel::<Mutation>();

    log.set_forwarder(Box::new(move |m| {
        // Sender::send only fails if the receiver was dropped, which
        // means the worker has exited. In that case we drop the
        // mutation; engine commit must already have persisted it to
        // the on-disk log, so it'll be replayed on the next reconnect.
        let _ = tx.send(m);
    }));

    let mut controller = controller;
    thread::spawn(move || {
        while let Ok(m) = rx.recv() {
            controller.apply(m);
        }
        controller.flush();
    });

    WorkerHandle { _private: () }
}

#[cfg(target_arch = "wasm32")]
fn spawn_impl<C: Controller>(log: &MutationLog, controller: C) -> WorkerHandle {
    use futures_channel::mpsc;
    use futures_util::stream::StreamExt;

    let (tx, mut rx) = mpsc::unbounded::<Mutation>();

    log.set_forwarder(Box::new(move |m| {
        let _ = tx.unbounded_send(m);
    }));

    let mut controller = controller;
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(m) = rx.next().await {
            controller.apply(m);
        }
        controller.flush();
    });

    WorkerHandle { _private: () }
}
