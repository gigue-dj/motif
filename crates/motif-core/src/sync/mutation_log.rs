//! `MutationLog` is the bridge between a local commit and the
//! `ControllerClient`. It assigns monotonic `local_seq` values and either
//! forwards to the wired client or buffers if none is wired yet.
//!
//! v0.0.1 keeps the log entirely in-process. v0.0.2 will persist it
//! alongside the storage engine so queued mutations survive crashes and
//! offline-mode restarts.

use std::sync::{Arc, Mutex};

use super::{ControllerClient, Mutation};

#[derive(Default)]
pub struct MutationLog {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    next_seq: u64,
    buffer: Vec<Mutation>,
    client: Option<Arc<dyn ControllerClient>>,
}

impl MutationLog {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next_seq: 1,
                buffer: Vec::new(),
                client: None,
            }),
        }
    }

    /// Record a mutation. The `local_seq` field of the input is overwritten
    /// with the next monotonic value. If a client is wired, the mutation
    /// is forwarded under the lock so the per-mutation order matches the
    /// `local_seq` order observed by the client.
    pub fn record(&self, mut m: Mutation) {
        let mut g = self.inner.lock().expect("poisoned");
        m.local_seq = g.next_seq;
        g.next_seq += 1;
        match g.client.clone() {
            Some(client) => client.apply_mutation(m),
            None => g.buffer.push(m),
        }
    }

    /// Wire a client. Pre-buffered mutations are NOT replayed automatically
    /// — the caller chooses whether to drain them via `take_buffer` and
    /// re-apply.
    pub fn set_client(&self, client: Arc<dyn ControllerClient>) {
        let mut g = self.inner.lock().expect("poisoned");
        g.client = Some(client);
    }

    /// Take everything currently buffered. Used for tests and for the
    /// alpha.5 WAL-replay startup path.
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
    use super::super::{ActorId, InMemoryControllerClient, MutationKind};
    use super::*;

    fn sample(kind: MutationKind) -> Mutation {
        Mutation {
            local_seq: 0, // overwritten by record()
            kind,
            actor: ActorId {
                user_id: "u".into(),
                device_id: "d".into(),
            },
            table_name: "T".into(),
            wal_payload: vec![],
        }
    }

    #[test]
    fn assigns_monotonic_seq_when_buffering() {
        let log = MutationLog::new();
        log.record(sample(MutationKind::NodeInsert));
        log.record(sample(MutationKind::NodeUpdate));
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
        log.record(sample(MutationKind::NodeInsert));
        log.record(sample(MutationKind::RelInsert));
        assert_eq!(log.buffered_len(), 0);
        let received = client.drain();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].local_seq, 1);
        assert_eq!(received[1].local_seq, 2);
    }

    #[test]
    fn seq_continues_after_wiring() {
        let log = MutationLog::new();
        log.record(sample(MutationKind::NodeInsert)); // seq 1, buffered
        let client = Arc::new(InMemoryControllerClient::new());
        log.set_client(client.clone());
        log.record(sample(MutationKind::NodeUpdate)); // seq 2, forwarded
        assert_eq!(log.buffered_len(), 1);
        assert_eq!(client.len(), 1);
        assert_eq!(client.drain()[0].local_seq, 2);
    }
}
