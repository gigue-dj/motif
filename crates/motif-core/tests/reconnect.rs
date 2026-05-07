//! End-to-end test for v0.0.2-alpha.4 controller-worker retry +
//! backoff. A `FlakyController` fails the first N applies with
//! `Transient`; the worker retries with exponential backoff and the
//! mutation eventually lands.
//!
//! This is the architectural-validation test for the reconnect /
//! offline state machine: real bridges (SurrealDB / Supabase / etc.)
//! will signal `Transient` whenever the network drops; the worker
//! handles retries entirely; the host doesn't see anything beyond
//! eventual mutation arrival.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use motif_core::{
    Controller, ControllerConfig, ControllerError, EdgeConfig, Engine, IdentityConfig, MotifConfig,
    Mutation, Node, StorageConfig,
};
use tempfile::TempDir;

/// Test fixture: a controller that fails the first `failures` apply
/// calls with `ControllerError::Transient`, then succeeds for the
/// rest. Records every successful apply into a shared Vec for
/// inspection.
#[derive(Clone)]
struct FlakyController {
    handle: Arc<Mutex<Vec<Mutation>>>,
    failures_remaining: Arc<AtomicU32>,
    connected: Arc<Mutex<bool>>,
}

impl FlakyController {
    fn new(initial_failures: u32) -> Self {
        Self {
            handle: Arc::default(),
            failures_remaining: Arc::new(AtomicU32::new(initial_failures)),
            connected: Arc::new(Mutex::new(false)),
        }
    }

    fn applied(&self) -> Vec<Mutation> {
        self.handle.lock().unwrap().clone()
    }

    fn was_connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }
}

impl Controller for FlakyController {
    fn connect(
        &mut self,
        _capability: &motif_core::CapabilityConfig,
    ) -> Result<(), ControllerError> {
        *self.connected.lock().unwrap() = true;
        Ok(())
    }

    fn apply(&mut self, m: &Mutation) -> Result<(), ControllerError> {
        // Only decrement while above zero (avoids u32 underflow into
        // u32::MAX, which would re-trip the failure path forever).
        let n = self.failures_remaining.load(Ordering::SeqCst);
        if n > 0 {
            self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(ControllerError::transient("simulated transient"));
        }
        self.handle.lock().unwrap().push(m.clone());
        Ok(())
    }
}

fn config_for_tests(path: PathBuf) -> MotifConfig {
    MotifConfig {
        identity: IdentityConfig {
            user_id: "u_test".into(),
            device_id: "d_test".into(),
        },
        controller: ControllerConfig {
            kind: "in-memory".into(),
        },
        storage: StorageConfig { path },
        capability: Default::default(),
        // Tight backoff cap so the test doesn't take forever waiting
        // for retries to converge.
        edge: EdgeConfig {
            controller_retry_max_backoff_ms: 50,
            ..Default::default()
        },
    }
}

fn wait_for_count(handle: &FlakyController, count: usize, timeout: Duration) -> Vec<Mutation> {
    let start = std::time::Instant::now();
    loop {
        let v = handle.applied();
        if v.len() >= count {
            return v;
        }
        if start.elapsed() > timeout {
            panic!(
                "wait_for_count: timeout after {:?} waiting for {count} (got {})",
                timeout,
                v.len()
            );
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn worker_calls_connect_before_apply() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("connect.db");
    let cfg = config_for_tests(path);

    let controller = FlakyController::new(0); // never fails
    let inspector = controller.clone();
    let mut e = Engine::open(&cfg).unwrap().with_controller(controller);

    // Submit one mutation; the worker must connect first, then apply.
    e.insert_node(Node::new("a", "Person")).unwrap();
    let _ = wait_for_count(&inspector, 1, Duration::from_secs(2));
    assert!(inspector.was_connected());
}

#[test]
fn worker_retries_transient_failures() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("retry.db");
    let cfg = config_for_tests(path);

    // Fail the first 3 apply attempts; the 4th succeeds.
    let controller = FlakyController::new(3);
    let inspector = controller.clone();
    let mut e = Engine::open(&cfg).unwrap().with_controller(controller);

    e.insert_node(Node::new("eventually-lands", "Person"))
        .unwrap();

    // Default backoff: 100, 200, 400ms (capped at 50ms by our test
    // config). Total wait ~150ms; allow 5s for slow CI.
    let received = wait_for_count(&inspector, 1, Duration::from_secs(5));
    assert_eq!(received.len(), 1);
    match &received[0].op {
        motif_core::MutationOp::NodeInsert(n) => assert_eq!(n.id, "eventually-lands"),
        other => panic!("expected NodeInsert, got {other:?}"),
    }
}

#[test]
fn worker_retries_burst_under_transient_failures() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("burst.db");
    let cfg = config_for_tests(path);

    // Fail the first 2 apply attempts (across the whole burst); after
    // that everything succeeds. Total apply calls = N + 2.
    let controller = FlakyController::new(2);
    let inspector = controller.clone();
    let mut e = Engine::open(&cfg).unwrap().with_controller(controller);

    const N: usize = 20;
    for i in 0..N {
        e.insert_node(Node::new(format!("n{i}"), "Person")).unwrap();
    }

    let received = wait_for_count(&inspector, N, Duration::from_secs(5));
    assert_eq!(received.len(), N);
    // local_seq order preserved across the retry storm.
    for (i, m) in received.iter().enumerate() {
        assert_eq!(m.local_seq, (i + 1) as u64);
    }
}

#[test]
fn permanent_error_drops_mutation_but_does_not_kill_worker() {
    // A controller that returns Permanent on the first apply, then OK
    // for the rest. The first mutation is dropped; the second lands.
    #[derive(Clone)]
    struct PermThenOk {
        handle: Arc<Mutex<Vec<Mutation>>>,
        seen: Arc<AtomicU32>,
    }
    impl Controller for PermThenOk {
        fn apply(&mut self, m: &Mutation) -> Result<(), ControllerError> {
            let n = self.seen.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                return Err(ControllerError::permanent("schema mismatch"));
            }
            self.handle.lock().unwrap().push(m.clone());
            Ok(())
        }
    }

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("perm.db");
    let cfg = config_for_tests(path);
    let received: Arc<Mutex<Vec<Mutation>>> = Arc::default();
    let controller = PermThenOk {
        handle: received.clone(),
        seen: Arc::new(AtomicU32::new(0)),
    };
    let mut e = Engine::open(&cfg).unwrap().with_controller(controller);

    e.insert_node(Node::new("dropped", "Person")).unwrap();
    e.insert_node(Node::new("kept", "Person")).unwrap();

    // Wait for the second to land.
    let start = std::time::Instant::now();
    while received.lock().unwrap().is_empty() && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(2));
    }
    let v = received.lock().unwrap().clone();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].op.target_id(), Some("kept"));
}
