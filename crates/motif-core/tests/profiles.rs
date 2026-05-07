//! Test profiles per MOTIF.md decision 22. v0.0.2-alpha.4 stands up
//! `default` (no injection) and `potato` (artificially throttled
//! storage I/O); `hoverphone` (artificially over-fast / unusual
//! timing) is deferred to v0.0.3+ — it requires a more invasive
//! scheduling primitive than a sleep-per-op wrapper.
//!
//! The same engine-level test (a 30-mutation insert burst with the
//! controller worker observing `local_seq` ordering) runs under both
//! profiles. If the test passes under both, we know the system
//! tolerates I/O slowness without losing ordering.

use std::path::PathBuf;
use std::time::Duration;

use motif_core::{
    ControllerConfig, Engine, IdentityConfig, InMemoryController, MotifConfig, Node, Storage,
    StorageConfig, StorageError,
};

/// Wraps a `Storage` impl with a fixed delay before each operation.
/// The "potato" profile sleeps a few hundred microseconds per op to
/// simulate slow flash on a constrained device.
struct ThrottledStorage<S: Storage> {
    inner: S,
    delay: Duration,
}

impl<S: Storage> ThrottledStorage<S> {
    fn new(inner: S, delay: Duration) -> Self {
        Self { inner, delay }
    }
}

impl<S: Storage> Storage for ThrottledStorage<S> {
    fn append(&mut self, bytes: &[u8]) -> Result<u64, StorageError> {
        std::thread::sleep(self.delay);
        self.inner.append(bytes)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, StorageError> {
        std::thread::sleep(self.delay);
        self.inner.read_at(offset, len)
    }

    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn truncate(&mut self, new_len: u64) -> Result<(), StorageError> {
        self.inner.truncate(new_len)
    }
}

fn config_for_tests(path: PathBuf) -> MotifConfig {
    MotifConfig {
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

fn run_burst_with_storage(storage: Box<dyn Storage>, burst: usize, timeout: Duration) {
    let cfg = config_for_tests(PathBuf::from(":memory:"));
    let controller = InMemoryController::new();
    let handle = controller.handle();
    let mut e = Engine::open_with(&cfg, storage)
        .unwrap()
        .with_controller(controller);

    for i in 0..burst {
        e.insert_node(Node::new(format!("n{i}"), "Person")).unwrap();
    }

    let received = handle.wait_for(burst, timeout);
    assert_eq!(received.len(), burst);
    // local_seq order preserved across the burst.
    for (i, m) in received.iter().enumerate() {
        assert_eq!(m.local_seq, (i + 1) as u64);
    }
}

#[test]
fn burst_default_profile() {
    run_burst_with_storage(
        Box::new(motif_core::MemoryStorage::new()),
        30,
        Duration::from_secs(2),
    );
}

#[test]
fn burst_potato_profile() {
    let inner = motif_core::MemoryStorage::new();
    let throttled = ThrottledStorage::new(inner, Duration::from_micros(500));
    // 30 inserts × ~3 storage ops × 500us ≈ 45ms expected, plus
    // controller worker overhead. Use a generous 5s timeout for slow
    // CI runners.
    run_burst_with_storage(Box::new(throttled), 30, Duration::from_secs(5));
}
