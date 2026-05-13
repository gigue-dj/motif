//! Architectural test: every committed engine mutation is teed to the
//! controller worker. v0.0.2-alpha.2 asserts that the worker thread
//! actually runs (mutations are visible from the test thread via
//! `InMemoryHandle::wait_for`), and that the foreshadow flag is set on
//! every fresh commit.

use std::path::PathBuf;
use std::time::Duration;

use morceau_core::{
    ControllerConfig, Engine, IdentityConfig, InMemoryController, MorceauConfig, MutationOp, Node,
    Params, StorageConfig, Value,
};
use tempfile::TempDir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(2);

fn config() -> MorceauConfig {
    MorceauConfig {
        identity: IdentityConfig {
            user_id: "u_test".into(),
            device_id: "d_test".into(),
        },
        controller: ControllerConfig {
            kind: "in-memory".into(),
        },
        storage: StorageConfig {
            path: PathBuf::from(":memory:"),
        },
        capability: Default::default(),
        edge: Default::default(),
    }
}

fn engine_with_controller() -> (TempDir, Engine, morceau_core::InMemoryHandle) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hook.db");
    let cfg = MorceauConfig {
        storage: StorageConfig { path },
        ..config()
    };
    let controller = InMemoryController::new();
    let handle = controller.handle();
    let engine = Engine::open(&cfg).unwrap().with_controller(controller);
    (dir, engine, handle)
}

#[test]
fn every_insert_tees_a_mutation_to_the_worker() {
    let (_dir, mut e, handle) = engine_with_controller();

    e.query(
        "CREATE (n:Person {id: 'alice', name: 'Alice'})",
        &Params::new(),
    )
    .unwrap();
    e.query("CREATE (n:Person {id: 'bob'})", &Params::new())
        .unwrap();

    let received = handle.wait_for(2, WAIT_TIMEOUT);
    assert_eq!(received.len(), 2);
    assert!(received[0].foreshadow);
    assert!(received[1].foreshadow);
    match &received[0].op {
        MutationOp::NodeInsert(n) => {
            assert_eq!(n.id, "alice");
            assert_eq!(n.label, "Person");
        }
        other => panic!("expected NodeInsert, got {other:?}"),
    }
    assert_eq!(received[0].actor.user_id, "u_test");
    assert_eq!(received[0].actor.device_id, "d_test");
    assert_eq!(received[0].local_seq, 1);
    assert_eq!(received[1].local_seq, 2);
}

#[test]
fn delete_tees_a_mutation() {
    let (_dir, mut e, handle) = engine_with_controller();

    e.insert_node(Node::new("doomed", "Person")).unwrap();
    let _ = handle.wait_for(1, WAIT_TIMEOUT);
    let _ = handle.drain();

    e.delete_node("doomed").unwrap();
    let received = handle.wait_for(1, WAIT_TIMEOUT);
    assert_eq!(received.len(), 1);
    assert!(received[0].foreshadow);
    match &received[0].op {
        MutationOp::NodeDelete(id) => assert_eq!(id, "doomed"),
        other => panic!("expected NodeDelete, got {other:?}"),
    }
}

#[test]
fn no_mutation_emitted_for_failed_operations() {
    let (_dir, mut e, handle) = engine_with_controller();
    e.query("CREATE (n:Person {id: 'a'})", &Params::new())
        .unwrap();
    let _ = handle.wait_for(1, WAIT_TIMEOUT);
    let _ = handle.drain();

    // Duplicate id should error and emit no mutation.
    let err = e.query("CREATE (n:Person {id: 'a'})", &Params::new());
    assert!(err.is_err());
    // Give the worker a moment to (not) drain anything new.
    std::thread::sleep(Duration::from_millis(50));
    assert!(handle.is_empty());
}

#[test]
fn engine_without_controller_still_works() {
    let dir = TempDir::new().unwrap();
    let cfg = MorceauConfig {
        storage: StorageConfig {
            path: dir.path().join("nocontroller.db"),
        },
        ..config()
    };
    let mut e = Engine::open(&cfg).unwrap();
    e.insert_node(Node::new("n", "Person").with_property("v", Value::I64(1)))
        .unwrap();
    assert!(e.get_node("n").unwrap().is_some());
    // No controller wired → no MutationLog → buffered count is 0.
    assert_eq!(e.buffered_mutation_count(), 0);
}

#[test]
fn worker_preserves_local_seq_order_under_burst() {
    // Submit a burst of writes; the worker should observe them in the
    // same local_seq order the engine assigned. Sanity-checks the
    // single-channel-per-controller invariant.
    let (_dir, mut e, handle) = engine_with_controller();

    const N: usize = 50;
    for i in 0..N {
        let id = format!("n{i}");
        e.insert_node(Node::new(&id, "Person")).unwrap();
    }

    let received = handle.wait_for(N, WAIT_TIMEOUT);
    assert_eq!(received.len(), N);
    for (i, m) in received.iter().enumerate() {
        assert_eq!(m.local_seq, (i + 1) as u64);
    }
}
