//! `ControllerClient` is the only part of Motif that knows about the
//! existence of an upstream authority. v0.0.1 ships only the in-memory
//! implementation; v0.0.2 swaps in a SurrealDB transport behind the same
//! trait without touching any other crate.

use std::sync::Mutex;

use super::Mutation;

/// Receiver for committed local mutations. Implementations must be safe to
/// call from a transaction-commit hot path: queue and return, never block
/// on network I/O.
pub trait ControllerClient: Send + Sync {
    fn apply_mutation(&self, m: Mutation);

    /// Best-effort flush of any queued mutations. v0.0.1 is a no-op; v0.0.2
    /// will actually push to SurrealDB.
    fn flush(&self);
}

/// Default v0.0.1 implementation: a thread-safe in-memory queue. Used as
/// the destination for the (still-unwired) WAL commit hook so we can
/// validate the architecture end-to-end without any network code. The
/// queue is exposed via `drain` for test inspection.
#[derive(Debug, Default)]
pub struct InMemoryControllerClient {
    queue: Mutex<Vec<Mutation>>,
}

impl InMemoryControllerClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test/inspection helper: take everything that has been buffered.
    pub fn drain(&self) -> Vec<Mutation> {
        let mut g = self.queue.lock().expect("poisoned");
        std::mem::take(&mut *g)
    }

    pub fn len(&self) -> usize {
        self.queue.lock().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ControllerClient for InMemoryControllerClient {
    fn apply_mutation(&self, m: Mutation) {
        self.queue.lock().expect("poisoned").push(m);
    }

    fn flush(&self) {
        // No upstream in v0.0.1; nothing to flush.
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ActorId, MutationOp};
    use super::*;
    use crate::graph::Node;

    fn sample_mutation(seq: u64) -> Mutation {
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
    fn buffers_and_drains() {
        let c = InMemoryControllerClient::new();
        assert!(c.is_empty());
        c.apply_mutation(sample_mutation(1));
        c.apply_mutation(sample_mutation(2));
        assert_eq!(c.len(), 2);
        let drained = c.drain();
        assert_eq!(drained.len(), 2);
        assert!(c.is_empty());
    }
}
