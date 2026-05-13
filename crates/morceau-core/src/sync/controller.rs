//! `Controller` is the only seam in morceau-core that knows about the
//! existence of an upstream authority. Concrete controller transports
//! live as optional separate `morceau-*-bridge` crates per `MORCEAU.md`
//! decision 18; morceau-core ships only the abstract trait and an
//! `InMemoryController` for tests and for the v0.0.2-alpha.2
//! threading-model validation.
//!
//! The controller is invoked from a dedicated worker thread (native) or
//! a `wasm-bindgen-futures::spawn_local` task (wasm). See `worker.rs`
//! for the spawning code. v0.0.2-alpha.4 enriched the trait with a
//! `connect()` lifecycle hook (called once at worker startup, given the
//! [`crate::config::CapabilityConfig`]) and made `apply()` return
//! `Result<(), ControllerError>` so bridges can signal transient
//! failures the worker should retry.

#[cfg(feature = "in-memory-controller")]
use std::sync::Mutex;

use super::Mutation;
use crate::config::CapabilityConfig;

/// Failure mode reported by a `Controller`. Determines the worker's
/// retry policy: `Transient` errors are retried with exponential
/// backoff; `Permanent` errors are dropped after logging (the
/// underlying mutation stays foreshadow=true on disk and can be
/// replayed via the alpha.5 `replay_unconfirmed` path).
#[derive(Debug, thiserror::Error)]
pub enum ControllerError {
    #[error("transient controller failure: {reason}")]
    Transient { reason: String },
    #[error("permanent controller failure: {reason}")]
    Permanent { reason: String },
}

impl ControllerError {
    pub fn transient(reason: impl Into<String>) -> Self {
        ControllerError::Transient {
            reason: reason.into(),
        }
    }
    pub fn permanent(reason: impl Into<String>) -> Self {
        ControllerError::Permanent {
            reason: reason.into(),
        }
    }

    pub fn is_transient(&self) -> bool {
        matches!(self, ControllerError::Transient { .. })
    }
}

/// Receiver for committed local mutations. Implementations are owned
/// by the worker that the engine spawns when `Engine::with_controller`
/// is called.
///
/// `Controller` is `Send + 'static` because the worker takes ownership
/// across the thread / task boundary.
///
/// Lifecycle (driven by the worker):
/// 1. `connect(capability)` — once at startup. Bridges should
///    establish their network connection here. Default is a no-op
///    (in-memory and stub controllers don't need a connect step).
/// 2. `apply(&Mutation)` — once per committed mutation, in `local_seq`
///    order. May return [`ControllerError::Transient`] to request a
///    retry, or [`ControllerError::Permanent`] to give up on this
///    specific mutation.
/// 3. `flush()` — once at shutdown. Default is a no-op.
pub trait Controller: Send + 'static {
    /// Establish the controller-side connection. Called once at
    /// worker startup, before any `apply` calls. Default impl is a
    /// no-op for in-memory / stub controllers.
    ///
    /// `capability` carries the host's reported facts (RAM, cores,
    /// arch, etc.) per MORCEAU.md decision 20. Bridges that route
    /// based on capability use it here; others ignore.
    fn connect(&mut self, _capability: &CapabilityConfig) -> Result<(), ControllerError> {
        Ok(())
    }

    /// Apply a mutation that was committed locally. Called once per
    /// committed mutation, in `local_seq` order, on the worker thread.
    /// Takes `&Mutation` (not by value) so the worker can retry on
    /// transient failures without forcing the controller to clone.
    fn apply(&mut self, m: &Mutation) -> Result<(), ControllerError>;

    /// Best-effort flush. The worker calls this once when shutting
    /// down (channel sender dropped → recv returns EOF). Default is a
    /// no-op so most controllers don't have to think about it.
    fn flush(&mut self) {}
}

/// Default in-memory controller for tests and for the alpha.2
/// threading-model validation. Records every applied mutation into a
/// shared `Vec` that test code can inspect via [`InMemoryController::handle`].
///
/// The handle is `Clone + Send + Sync` so test code can hand it to the
/// engine (which takes ownership of the controller) and still inspect
/// results from the test thread.
///
/// Gated behind the `in-memory-controller` feature (default-on) so
/// production builds that wire a real bridge can drop this code via
/// `default-features = false`.
#[cfg(feature = "in-memory-controller")]
#[derive(Debug, Default)]
pub struct InMemoryController {
    handle: InMemoryHandle,
}

#[cfg(feature = "in-memory-controller")]
#[derive(Clone, Debug, Default)]
pub struct InMemoryHandle {
    state: std::sync::Arc<Mutex<Vec<Mutation>>>,
}

#[cfg(feature = "in-memory-controller")]
impl InMemoryHandle {
    /// Snapshot the mutations the worker has applied so far.
    pub fn snapshot(&self) -> Vec<Mutation> {
        self.state.lock().expect("poisoned").clone()
    }

    pub fn len(&self) -> usize {
        self.state.lock().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drain everything and return it (clears the buffer). Useful when
    /// a test wants to consume in batches.
    pub fn drain(&self) -> Vec<Mutation> {
        std::mem::take(&mut *self.state.lock().expect("poisoned"))
    }

    /// Block the calling thread until at least `count` mutations have
    /// been applied or `timeout` elapses. Polls every 1 ms.
    ///
    /// Panics on timeout — tests should set a generous timeout (1s+)
    /// because CI machines can be slow under load.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_for(&self, count: usize, timeout: std::time::Duration) -> Vec<Mutation> {
        let start = std::time::Instant::now();
        loop {
            let snap = self.snapshot();
            if snap.len() >= count {
                return snap;
            }
            if start.elapsed() > timeout {
                panic!(
                    "InMemoryHandle::wait_for: timeout after {:?} waiting for {count} mutations \
                     (got {})",
                    timeout,
                    snap.len()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

#[cfg(feature = "in-memory-controller")]
impl InMemoryController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-side handle. Hand `self` to the engine (which moves it
    /// onto the worker), keep the handle for inspection.
    pub fn handle(&self) -> InMemoryHandle {
        self.handle.clone()
    }
}

#[cfg(feature = "in-memory-controller")]
impl Controller for InMemoryController {
    fn apply(&mut self, m: &Mutation) -> Result<(), ControllerError> {
        self.handle.state.lock().expect("poisoned").push(m.clone());
        Ok(())
    }
}

#[cfg(all(test, feature = "in-memory-controller"))]
mod tests {
    use super::super::{ActorId, MutationOp};
    use super::*;
    use crate::graph::Node;

    fn sample(seq: u64) -> Mutation {
        Mutation {
            local_seq: seq,
            actor: ActorId {
                user_id: "u".into(),
                device_id: "d".into(),
            },
            foreshadow: true,
            op: MutationOp::NodeInsert(Node::new(format!("n{seq}"), "Person")),
        }
    }

    #[test]
    fn handle_observes_applied_mutations() {
        let mut c = InMemoryController::new();
        let h = c.handle();
        assert!(h.is_empty());
        c.apply(&sample(1)).unwrap();
        c.apply(&sample(2)).unwrap();
        let snap = h.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].local_seq, 1);
        assert_eq!(snap[1].local_seq, 2);
    }

    #[test]
    fn drain_clears_state() {
        let mut c = InMemoryController::new();
        let h = c.handle();
        c.apply(&sample(1)).unwrap();
        let drained = h.drain();
        assert_eq!(drained.len(), 1);
        assert!(h.is_empty());
    }

    #[test]
    fn default_connect_is_ok() {
        let mut c = InMemoryController::new();
        let cap = CapabilityConfig::default();
        c.connect(&cap).unwrap();
    }

    #[test]
    fn controller_error_helpers() {
        let t = ControllerError::transient("network down");
        assert!(t.is_transient());
        let p = ControllerError::permanent("auth failed");
        assert!(!p.is_transient());
    }
}
