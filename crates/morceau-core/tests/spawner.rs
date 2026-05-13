//! v0.0.3-alpha.2: integration coverage for the native `Spawner` seam.
//!
//! - A custom spawner *is actually invoked* when a host wires it via
//!   `Engine::with_controller_spawned_by`. Closes the trivial-but-
//!   important "we exposed the seam but never check it gets called"
//!   regression risk.
//! - Mutations still pump end-to-end through the host-supplied
//!   spawner — same correctness contract as the default
//!   `StdThreadSpawner`. Uses `InMemoryController` + its handle for
//!   the assertion.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use morceau_core::{
    ControllerConfig, Engine, IdentityConfig, InMemoryController, MorceauConfig, Node, Spawner,
    StorageConfig,
};
use tempfile::TempDir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(2);

fn config_with(path: PathBuf) -> MorceauConfig {
    MorceauConfig {
        identity: IdentityConfig {
            user_id: "u".into(),
            device_id: "d".into(),
        },
        controller: ControllerConfig {
            kind: "in-memory".into(),
        },
        storage: StorageConfig { path },
        capability: Default::default(),
        edge: Default::default(),
    }
}

/// Counts spawn calls and runs the closure on a regular `std::thread`
/// — same effect as `StdThreadSpawner`, with a counter on the side so
/// the test can assert the host-supplied spawner was actually invoked.
#[derive(Clone, Default)]
struct CountingSpawner {
    calls: Arc<AtomicUsize>,
}

impl Spawner for CountingSpawner {
    fn spawn_worker(&self, f: Box<dyn FnOnce() + Send + 'static>) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(f);
    }
}

#[test]
fn host_supplied_spawner_is_invoked() {
    let dir = TempDir::new().unwrap();
    let cfg = config_with(dir.path().join("spawner.db"));
    let controller = InMemoryController::new();
    let handle = controller.handle();
    let spawner = CountingSpawner::default();
    let calls = spawner.calls.clone();

    let mut e = Engine::open(&cfg)
        .unwrap()
        .with_controller_spawned_by(controller, spawner);

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "with_controller_spawned_by should have spawned the worker exactly once"
    );

    e.insert_node(Node::new("a", "Person")).unwrap();
    let received = handle.wait_for(1, WAIT_TIMEOUT);
    assert_eq!(received.len(), 1);
}

#[test]
fn host_supplied_spawner_pumps_burst_in_order() {
    let dir = TempDir::new().unwrap();
    let cfg = config_with(dir.path().join("spawner_burst.db"));
    let controller = InMemoryController::new();
    let handle = controller.handle();
    let spawner = CountingSpawner::default();

    let mut e = Engine::open(&cfg)
        .unwrap()
        .with_controller_spawned_by(controller, spawner);

    const N: usize = 25;
    for i in 0..N {
        e.insert_node(Node::new(format!("n{i}"), "Person")).unwrap();
    }

    let received = handle.wait_for(N, WAIT_TIMEOUT);
    assert_eq!(received.len(), N);
    for (i, m) in received.iter().enumerate() {
        assert_eq!(m.local_seq, (i + 1) as u64);
    }
}
