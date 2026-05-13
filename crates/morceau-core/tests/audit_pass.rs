//! v0.0.2-alpha.5 audit-pass coverage: validates the two new Engine
//! APIs that close PR #1 review debt.
//!
//! - `Engine::with_named_controller(c, kind)` — verifies the host
//!   wired the controller it declared in `MorceauConfig::controller.kind`.
//!   Closes the unvalidated-`kind` debt logged in PR #1.
//! - `Engine::replay_unconfirmed()` — re-feeds every foreshadow=true
//!   mutation from the persisted log to the wired controller. Used
//!   by hosts that crashed mid-task and want to catch the controller
//!   up on whatever was committed locally but not yet seen upstream.
//!   Closes the replay-from-disk gap penciled into v0.0.2-alpha.4.

use std::path::PathBuf;
use std::time::Duration;

use morceau_core::{
    ControllerConfig, Engine, EngineError, IdentityConfig, InMemoryController, MorceauConfig, Node,
    StorageConfig,
};
use tempfile::TempDir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(2);

fn config_with(path: PathBuf, kind: &str) -> MorceauConfig {
    MorceauConfig {
        identity: IdentityConfig {
            user_id: "u".into(),
            device_id: "d".into(),
        },
        controller: ControllerConfig {
            kind: kind.to_string(),
        },
        storage: StorageConfig { path },
        capability: Default::default(),
        edge: Default::default(),
    }
}

#[test]
fn with_named_controller_accepts_matching_kind() {
    let dir = TempDir::new().unwrap();
    let cfg = config_with(dir.path().join("named.db"), "in-memory");
    let controller = InMemoryController::new();
    let handle = controller.handle();
    let mut e = Engine::open(&cfg)
        .unwrap()
        .with_named_controller(controller, "in-memory")
        .unwrap();

    e.insert_node(Node::new("a", "Person")).unwrap();
    let _ = handle.wait_for(1, WAIT_TIMEOUT);
}

#[test]
fn with_named_controller_rejects_mismatched_kind() {
    let dir = TempDir::new().unwrap();
    let cfg = config_with(dir.path().join("mismatched.db"), "in-memory");
    let engine = Engine::open(&cfg).unwrap();

    let result = engine.with_named_controller(InMemoryController::new(), "surreal");
    match result {
        Err(EngineError::ControllerKindMismatch { declared, wired }) => {
            assert_eq!(declared, "in-memory");
            assert_eq!(wired, "surreal");
        }
        Ok(_) => panic!("expected ControllerKindMismatch"),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn replay_unconfirmed_with_no_controller_is_a_noop() {
    // The engine has no MutationLog wired (no controller spawned),
    // so replay_unconfirmed walks the disk but doesn't try to push
    // anywhere — returns 0.
    let dir = TempDir::new().unwrap();
    let cfg = config_with(dir.path().join("no_controller.db"), "in-memory");
    let mut e = Engine::open(&cfg).unwrap();
    e.insert_node(Node::new("a", "Person")).unwrap();
    e.insert_node(Node::new("b", "Person")).unwrap();
    assert_eq!(e.replay_unconfirmed().unwrap(), 0);
}

#[test]
fn replay_unconfirmed_re_feeds_persisted_mutations() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("replay.db");

    // First session: commit some mutations without wiring a controller.
    // They land on disk as foreshadow=true but go nowhere upstream.
    {
        let cfg = config_with(path.clone(), "in-memory");
        let mut e = Engine::open(&cfg).unwrap();
        e.insert_node(Node::new("a", "Person")).unwrap();
        e.insert_node(Node::new("b", "Person")).unwrap();
        e.insert_node(Node::new("c", "Person")).unwrap();
    }

    // Second session: open the persisted store, wire a fresh
    // in-memory controller, call replay_unconfirmed.
    let cfg = config_with(path, "in-memory");
    let controller = InMemoryController::new();
    let handle = controller.handle();
    let mut e = Engine::open(&cfg).unwrap().with_controller(controller);

    let replayed = e.replay_unconfirmed().unwrap();
    assert_eq!(replayed, 3);

    let received = handle.wait_for(3, WAIT_TIMEOUT);
    assert_eq!(received.len(), 3);
    // local_seq order is preserved (we walk the log in offset order).
    for (i, m) in received.iter().enumerate() {
        assert_eq!(m.local_seq, (i + 1) as u64);
    }
}

#[test]
fn replay_unconfirmed_walks_through_inserts_and_deletes_in_order() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("replay_with_deletes.db");

    {
        let cfg = config_with(path.clone(), "in-memory");
        let mut e = Engine::open(&cfg).unwrap();
        e.insert_node(Node::new("a", "Person")).unwrap();
        e.insert_node(Node::new("b", "Person")).unwrap();
        e.delete_node("a").unwrap();
    }

    let cfg = config_with(path, "in-memory");
    let controller = InMemoryController::new();
    let handle = controller.handle();
    let mut e = Engine::open(&cfg).unwrap().with_controller(controller);

    // 3 records on disk: 2 inserts + 1 delete. All foreshadow=true.
    let replayed = e.replay_unconfirmed().unwrap();
    assert_eq!(replayed, 3);

    let received = handle.wait_for(3, WAIT_TIMEOUT);
    assert_eq!(received.len(), 3);
    use morceau_core::MutationOp;
    matches!(received[0].op, MutationOp::NodeInsert(_));
    matches!(received[1].op, MutationOp::NodeInsert(_));
    matches!(received[2].op, MutationOp::NodeDelete(_));
}
