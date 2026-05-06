//! Architectural test: every committed engine mutation is teed to the
//! wired `MutationLog` and forwarded to the wired `ControllerClient`.
//! v0.0.2-alpha.1 also asserts that fresh mutations carry foreshadow=true.

use std::path::PathBuf;
use std::sync::Arc;

use motif_core::{
    ControllerConfig, ControllerKind, Engine, IdentityConfig, InMemoryControllerClient,
    MotifConfig, MutationLog, MutationOp, Node, Params, StorageConfig, Value,
};
use tempfile::TempDir;

fn config() -> MotifConfig {
    MotifConfig {
        identity: IdentityConfig {
            user_id: "u_test".into(),
            device_id: "d_test".into(),
        },
        controller: ControllerConfig {
            kind: ControllerKind::InMemory,
        },
        storage: StorageConfig {
            path: PathBuf::from(":memory:"),
        },
    }
}

fn engine_with_log() -> (
    TempDir,
    Engine,
    Arc<MutationLog>,
    Arc<InMemoryControllerClient>,
) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hook.db");
    let cfg = MotifConfig {
        storage: StorageConfig { path },
        ..config()
    };
    let log = Arc::new(MutationLog::new());
    let client = Arc::new(InMemoryControllerClient::new());
    log.set_client(client.clone());
    let engine = Engine::open(&cfg).unwrap().with_mutation_log(log.clone());
    (dir, engine, log, client)
}

#[test]
fn every_insert_tees_a_mutation_to_the_log() {
    let (_dir, mut e, _log, client) = engine_with_log();

    e.query(
        "CREATE (n:Person {id: 'alice', name: 'Alice'})",
        &Params::new(),
    )
    .unwrap();
    e.query("CREATE (n:Person {id: 'bob'})", &Params::new())
        .unwrap();

    let received = client.drain();
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
    let (_dir, mut e, _log, client) = engine_with_log();

    e.insert_node(Node::new("doomed", "Person")).unwrap();
    let _ = client.drain();

    e.delete_node("doomed").unwrap();
    let received = client.drain();
    assert_eq!(received.len(), 1);
    assert!(received[0].foreshadow);
    match &received[0].op {
        MutationOp::NodeDelete(id) => assert_eq!(id, "doomed"),
        other => panic!("expected NodeDelete, got {other:?}"),
    }
}

#[test]
fn no_mutation_emitted_for_failed_operations() {
    let (_dir, mut e, _log, client) = engine_with_log();
    e.query("CREATE (n:Person {id: 'a'})", &Params::new())
        .unwrap();
    let _ = client.drain();

    // Duplicate id should error and emit no mutation.
    let err = e.query("CREATE (n:Person {id: 'a'})", &Params::new());
    assert!(err.is_err());
    assert!(client.is_empty());
}

#[test]
fn engine_without_log_still_works() {
    let dir = TempDir::new().unwrap();
    let cfg = MotifConfig {
        storage: StorageConfig {
            path: dir.path().join("nolog.db"),
        },
        ..config()
    };
    let mut e = Engine::open(&cfg).unwrap();
    e.insert_node(Node::new("n", "Person").with_property("v", Value::I64(1)))
        .unwrap();
    assert!(e.get_node("n").unwrap().is_some());
}
