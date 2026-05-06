//! `MutationLog` is the in-memory bridge between an engine commit and a
//! wired [`ControllerClient`]. It either forwards mutations to a wired
//! client or buffers them until one is wired.
//!
//! As of v0.0.2-alpha.1, `local_seq` is assigned by the engine before
//! `record()` is called — the engine owns sequence numbering because
//! the on-disk Mutation log (which IS the persisted source of truth)
//! must agree with what the controller eventually sees. The MutationLog
//! is now purely an in-memory FIFO for the controller worker.

use std::sync::{Arc, Mutex};

use super::{ControllerClient, Mutation};

#[derive(Default)]
pub struct MutationLog {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    buffer: Vec<Mutation>,
    client: Option<Arc<dyn ControllerClient>>,
}

impl MutationLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an already-sequenced mutation. The caller is expected to
    /// have assigned `local_seq`; we don't touch it. If a client is
    /// wired, the mutation is forwarded under the lock so per-mutation
    /// order matches the `local_seq` order observed by the client.
    pub fn record(&self, m: Mutation) {
        let mut g = self.inner.lock().expect("poisoned");
        match g.client.clone() {
            Some(client) => client.apply_mutation(m),
            None => g.buffer.push(m),
        }
    }

    /// Wire a client. Pre-buffered mutations are NOT replayed
    /// automatically — the caller chooses whether to drain them via
    /// `take_buffer` and re-apply.
    pub fn set_client(&self, client: Arc<dyn ControllerClient>) {
        let mut g = self.inner.lock().expect("poisoned");
        g.client = Some(client);
    }

    /// Take everything currently buffered. Used by tests and by the
    /// alpha.2 reconnect path that replays buffered mutations after
    /// the controller worker comes back online.
    pub fn take_buffer(&self) -> Vec<Mutation> {
        let mut g = self.inner.lock().expect("poisoned");
        std::mem::take(&mut g.buffer)
    }

    pub fn buffered_len(&self) -> usize {
        self.inner.lock().expect("poisoned").buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ActorId, InMemoryControllerClient, MutationOp};
    use super::*;
    use crate::graph::Node;

    fn sample(seq: u64, op: MutationOp) -> Mutation {
        Mutation {
            local_seq: seq,
            actor: ActorId {
                user_id: "u".into(),
                device_id: "d".into(),
            },
            foreshadow: true,
            op,
        }
    }

    #[test]
    fn buffers_when_no_client_wired() {
        let log = MutationLog::new();
        log.record(sample(1, MutationOp::NodeInsert(Node::new("a", "T"))));
        log.record(sample(2, MutationOp::NodeInsert(Node::new("b", "T"))));
        let drained = log.take_buffer();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].local_seq, 1);
        assert_eq!(drained[1].local_seq, 2);
    }

    #[test]
    fn forwards_to_client_when_wired() {
        let client = Arc::new(InMemoryControllerClient::new());
        let log = MutationLog::new();
        log.set_client(client.clone());
        log.record(sample(1, MutationOp::NodeInsert(Node::new("a", "T"))));
        log.record(sample(
            2,
            MutationOp::EdgeInsert(crate::graph::Edge::new("e", "F", "a", "a")),
        ));
        assert_eq!(log.buffered_len(), 0);
        let received = client.drain();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].local_seq, 1);
        assert_eq!(received[1].local_seq, 2);
    }

    #[test]
    fn does_not_renumber_after_wiring() {
        let log = MutationLog::new();
        log.record(sample(1, MutationOp::NodeInsert(Node::new("a", "T"))));
        let client = Arc::new(InMemoryControllerClient::new());
        log.set_client(client.clone());
        log.record(sample(2, MutationOp::NodeInsert(Node::new("b", "T"))));
        assert_eq!(log.buffered_len(), 1);
        assert_eq!(client.len(), 1);
        assert_eq!(client.drain()[0].local_seq, 2);
    }
}
