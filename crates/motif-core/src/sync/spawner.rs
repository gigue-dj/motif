//! Native-target task-spawn abstraction. v0.0.3-alpha.2 introduces this
//! seam so hosts on iOS / Android (when alpha.4 lands `cdylib` for
//! those targets) can route the controller worker through their
//! preferred runtime — Grand Central Dispatch on iOS, kotlinx
//! coroutines on Android — instead of `std::thread::spawn`.
//!
//! Scope: native targets only. The wasm path still uses
//! `wasm_bindgen_futures::spawn_local` directly because the host's wasm
//! runtime *is* the spawner — overriding it would mean swapping the
//! microtask queue for something else, which isn't a real use case.
//! On wasm, this trait isn't even compiled.
//!
//! Default: [`StdThreadSpawner`] — a thin wrapper around
//! `std::thread::spawn`. That's what `Engine::with_controller` uses
//! when no spawner is supplied. Hosts override via
//! `Engine::with_controller_spawned_by(controller, my_spawner)`.

#![cfg(not(target_arch = "wasm32"))]

/// Spawn a long-running worker closure on the host's preferred
/// thread / queue / coroutine runtime.
///
/// The closure runs the controller worker loop (channel `recv` →
/// `Controller::apply` with retry/backoff) until the channel returns
/// EOF. Implementations are expected to detach the work — there is
/// no join / cancel semantics in v0.0.3-alpha.2. The worker stops
/// on its own when the engine drops the channel sender.
///
/// `Send + Sync + 'static` because [`Engine::with_controller_spawned_by`]
/// stores the spawner across configuration — a host might wire one
/// per process and reuse it.
pub trait Spawner: Send + Sync + 'static {
    /// Spawn `f` to run on whatever underlying runtime this spawner
    /// represents. The default `StdThreadSpawner` calls
    /// `std::thread::spawn`; an iOS GCD spawner would call
    /// `dispatch_async`; an Android coroutine spawner would
    /// `Dispatchers.IO.launch` (via JNI).
    ///
    /// `f` is `FnOnce() + Send + 'static` because the worker loop
    /// is single-shot: spawn once, run until EOF, exit.
    fn spawn_worker(&self, f: Box<dyn FnOnce() + Send + 'static>);
}

/// Default `Spawner`: each `spawn_worker` call creates a fresh OS
/// thread via `std::thread::spawn` and detaches it. Matches the
/// pre-alpha.2 behavior — no behavior change for callers that stay on
/// `Engine::with_controller`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdThreadSpawner;

impl Spawner for StdThreadSpawner {
    fn spawn_worker(&self, f: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Inline spawner: runs `f` synchronously on the calling thread.
    /// Useful for deterministic tests; not a real-runtime example.
    #[derive(Default)]
    struct InlineSpawner {
        calls: Arc<AtomicUsize>,
    }

    impl Spawner for InlineSpawner {
        fn spawn_worker(&self, f: Box<dyn FnOnce() + Send + 'static>) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            f();
        }
    }

    #[test]
    fn std_thread_spawner_actually_spawns() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();
        let s = StdThreadSpawner;
        s.spawn_worker(Box::new(move || tx.send(()).unwrap()));
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .expect("std::thread::spawn should run the closure");
    }

    #[test]
    fn custom_spawner_invocation_is_observed() {
        let spawner = InlineSpawner::default();
        let calls = spawner.calls.clone();
        spawner.spawn_worker(Box::new(|| {}));
        spawner.spawn_worker(Box::new(|| {}));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
