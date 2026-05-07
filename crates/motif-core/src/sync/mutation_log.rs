//! `MutationLog` is the in-memory bridge between an engine commit and a
//! controller worker. v0.0.2-alpha.2 reworks this into a forwarder
//! abstraction: when no forwarder is wired, `record()` buffers in
//! memory; when one is wired (typically a channel sender installed by
//! the worker), `record()` invokes it directly. The worker thread on
//! the other end of the channel calls `Controller::apply` outside the
//! engine's commit path, keeping commit latency low.
//!
//! `local_seq` is assigned by the engine before `record()` is called —
//! the engine owns sequence numbering because the on-disk Mutation log
//! (the persisted source of truth) must agree with what the controller
//! eventually sees.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::Mutation;

/// Forwarder closure type. Captures whatever channel sender the worker
/// uses on this target.
type Forwarder = Box<dyn Fn(Mutation) + Send + Sync>;

#[derive(Default)]
pub struct MutationLog {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    buffer: Vec<Mutation>,
    forwarder: Option<Forwarder>,
}

/// Lock the inner mutex, recovering gracefully from a poisoned state.
///
/// v0.0.2-alpha.5 closes PR #1 review finding 7: the previous
/// `.expect("poisoned")` propagated a panic into anyone calling
/// `MutationLog::record`, which on the engine commit path is a poor
/// failure mode. A poisoned mutex here means a previous holder
/// panicked; the data inside (a `Vec<Mutation>` plus an
/// `Option<Box<dyn Fn(_)>>`) is still structurally valid, just
/// possibly not in the state the panic expected. For the
/// MutationLog's append-only / forwarder-replace semantics it is
/// safe to recover from poison and keep going.
fn lock_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MutationLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an already-sequenced mutation. If a forwarder is wired,
    /// the mutation goes through it; otherwise it lands in the in-memory
    /// buffer (which tests / startup-replay code can drain).
    pub fn record(&self, m: Mutation) {
        let mut g = lock_recover(&self.inner);
        match &g.forwarder {
            Some(f) => f(m),
            None => g.buffer.push(m),
        }
    }

    /// Wire a forwarder. Pre-buffered mutations are NOT replayed — the
    /// caller chooses whether to drain them via `take_buffer` and
    /// re-apply.
    pub fn set_forwarder(&self, f: Forwarder) {
        lock_recover(&self.inner).forwarder = Some(f);
    }

    /// True iff a forwarder is currently wired. Useful for tests that
    /// want to assert the worker spawned successfully.
    pub fn has_forwarder(&self) -> bool {
        lock_recover(&self.inner).forwarder.is_some()
    }

    /// Take everything currently buffered. Used by tests and by the
    /// alpha.5 reconnect path that replays buffered mutations after the
    /// controller worker comes back online.
    pub fn take_buffer(&self) -> Vec<Mutation> {
        let mut g = lock_recover(&self.inner);
        std::mem::take(&mut g.buffer)
    }

    pub fn buffered_len(&self) -> usize {
        lock_recover(&self.inner).buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::{ActorId, MutationOp};
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
    fn buffers_when_no_forwarder_wired() {
        let log = MutationLog::new();
        log.record(sample(1, MutationOp::NodeInsert(Node::new("a", "T"))));
        log.record(sample(2, MutationOp::NodeInsert(Node::new("b", "T"))));
        let drained = log.take_buffer();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].local_seq, 1);
        assert_eq!(drained[1].local_seq, 2);
    }

    #[test]
    fn forwards_through_closure_when_wired() {
        let received: Arc<Mutex<Vec<Mutation>>> = Arc::default();
        let received_clone = Arc::clone(&received);
        let log = MutationLog::new();
        log.set_forwarder(Box::new(move |m| {
            received_clone.lock().unwrap().push(m);
        }));

        log.record(sample(1, MutationOp::NodeInsert(Node::new("a", "T"))));
        log.record(sample(
            2,
            MutationOp::EdgeInsert(crate::graph::Edge::new("e", "F", "a", "a")),
        ));

        let got = received.lock().unwrap().clone();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].local_seq, 1);
        assert_eq!(got[1].local_seq, 2);
        assert_eq!(log.buffered_len(), 0);
    }

    #[test]
    fn pre_existing_buffer_is_not_drained_on_wire() {
        let log = MutationLog::new();
        log.record(sample(1, MutationOp::NodeInsert(Node::new("a", "T"))));

        let received: Arc<Mutex<Vec<Mutation>>> = Arc::default();
        let received_clone = Arc::clone(&received);
        log.set_forwarder(Box::new(move |m| {
            received_clone.lock().unwrap().push(m);
        }));

        log.record(sample(2, MutationOp::NodeInsert(Node::new("b", "T"))));

        // Only the post-wire mutation reached the forwarder.
        let got = received.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].local_seq, 2);

        // The pre-wire mutation is still buffered.
        assert_eq!(log.buffered_len(), 1);
        let drained = log.take_buffer();
        assert_eq!(drained[0].local_seq, 1);
    }

    #[test]
    fn has_forwarder_reflects_state() {
        let log = MutationLog::new();
        assert!(!log.has_forwarder());
        log.set_forwarder(Box::new(|_| {}));
        assert!(log.has_forwarder());
    }

    #[test]
    fn record_recovers_from_poisoned_mutex() {
        // Simulate a previous holder panicking inside the lock — the
        // mutex is now poisoned. The post-alpha.5 lock_recover helper
        // should pick up the inner data and let subsequent record()
        // calls keep working. Closes PR #1 review finding 7.
        let log = std::sync::Arc::new(MutationLog::new());
        let log_for_panic = std::sync::Arc::clone(&log);
        let _ = std::thread::spawn(move || {
            log_for_panic.set_forwarder(Box::new(|_| {
                panic!("holder panicked");
            }));
            log_for_panic.record(sample(1, MutationOp::NodeInsert(Node::new("x", "T"))));
        })
        .join();

        // Worker thread panicked inside the forwarder closure; the
        // lock is now poisoned. record() must not panic — it must
        // recover the inner state and keep going. Replace the
        // forwarder first so subsequent record() doesn't re-panic.
        log.set_forwarder(Box::new(|_| {}));
        log.record(sample(2, MutationOp::NodeInsert(Node::new("y", "T"))));
    }
}
