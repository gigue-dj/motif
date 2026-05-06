//! `Controller` is the only seam in motif-core that knows about the
//! existence of an upstream authority. Concrete controller transports
//! live as optional separate `motif-*-bridge` crates per `MOTIF.md`
//! decision 18; motif-core ships only the abstract trait and an
//! `InMemoryController` for tests and for the v0.0.2-alpha.2
//! threading-model validation.
//!
//! The controller is invoked from a dedicated worker thread (native) or
//! a `wasm-bindgen-futures::spawn_local` task (wasm). See `worker.rs`
//! for the spawning code. `apply` therefore takes `&mut self` — the
//! controller is owned exclusively by the worker.

use std::sync::Mutex;

use super::Mutation;

/// Receiver for committed local mutations. Implementations are owned by
/// the worker that the engine spawns when `Engine::with_controller` is
/// called.
///
/// `Controller` is `Send + 'static` because the worker takes ownership
/// across the thread / task boundary.
pub trait Controller: Send + 'static {
    /// Apply a mutation that was committed locally. Called once per
    /// committed mutation, in `local_seq` order, on the worker thread.
    fn apply(&mut self, m: Mutation);

    /// Best-effort flush. v0.0.2-alpha.2 calls this when the worker is
    /// shutting down. Default is a no-op so most controllers don't have
    /// to think about it.
    fn flush(&mut self) {}
}

/// Default in-memory controller for tests and for the alpha.2
/// threading-model validation. Records every applied mutation into a
/// shared `Vec` that test code can inspect via [`InMemoryController::handle`].
///
/// The handle is `Clone + Send + Sync` so test code can hand it to the
/// engine (which takes ownership of the controller) and still inspect
/// results from the test thread.
#[derive(Debug, Default)]
pub struct InMemoryController {
    handle: InMemoryHandle,
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryHandle {
    state: std::sync::Arc<Mutex<Vec<Mutation>>>,
}

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

impl InMemoryController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-side handle. Hand `self` to the engine (which moves it onto
    /// the worker), keep the handle for inspection.
    pub fn handle(&self) -> InMemoryHandle {
        self.handle.clone()
    }
}

impl Controller for InMemoryController {
    fn apply(&mut self, m: Mutation) {
        self.handle.state.lock().expect("poisoned").push(m);
    }
}

#[cfg(test)]
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
        c.apply(sample(1));
        c.apply(sample(2));
        let snap = h.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].local_seq, 1);
        assert_eq!(snap[1].local_seq, 2);
    }

    #[test]
    fn drain_clears_state() {
        let mut c = InMemoryController::new();
        let h = c.handle();
        c.apply(sample(1));
        let drained = h.drain();
        assert_eq!(drained.len(), 1);
        assert!(h.is_empty());
    }
}
