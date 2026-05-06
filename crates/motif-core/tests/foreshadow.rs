//! End-to-end test for v0.0.2-alpha.1 foreshadow tracking and the
//! `_motif.foreshadow` Cypher metadata namespace (MOTIF.md decisions
//! 19, 20, 22).
//!
//! These tests cover the alpha.1 architectural-validation goals:
//! every committed mutation carries `foreshadow=true` until the (alpha.2)
//! controller flow lands; the persisted MutationLog survives reopen
//! preserving the foreshadow state; and Cypher `n._motif.foreshadow`
//! returns the right answer in both `WHERE` and `RETURN` positions.

use std::path::PathBuf;

use motif_core::{
    ControllerConfig, ControllerKind, Engine, IdentityConfig, MotifConfig, Node, Params,
    ResultCell, StorageConfig, Value,
};
use tempfile::TempDir;

fn config_with(path: PathBuf) -> MotifConfig {
    MotifConfig {
        identity: IdentityConfig {
            user_id: "u".into(),
            device_id: "d".into(),
        },
        controller: ControllerConfig {
            kind: ControllerKind::InMemory,
        },
        storage: StorageConfig { path },
    }
}

#[test]
fn fresh_inserts_are_foreshadowed_in_memory() {
    let cfg = MotifConfig {
        storage: StorageConfig {
            path: PathBuf::from(":memory:"),
        },
        ..config_with(PathBuf::from(":memory:"))
    };
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.insert_node(Node::new("a", "Person")).unwrap();
    e.insert_node(Node::new("b", "Person")).unwrap();
    assert!(e.is_foreshadow("a"));
    assert!(e.is_foreshadow("b"));
    assert!(!e.is_foreshadow("missing"));
}

#[test]
fn foreshadow_state_survives_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("fs.db");

    {
        let mut e = Engine::open(&config_with(path.clone())).unwrap();
        e.insert_node(Node::new("alice", "Person").with_property("age", Value::I64(30)))
            .unwrap();
        e.insert_node(Node::new("bob", "Person")).unwrap();
        assert!(e.is_foreshadow("alice"));
    }

    // Reopen: the persisted Mutation log replays into the index +
    // foreshadow tracker.
    let reopened = Engine::open(&config_with(path)).unwrap();
    assert!(reopened.is_foreshadow("alice"));
    assert!(reopened.is_foreshadow("bob"));
    assert_eq!(reopened.node_count(), 2);
}

#[test]
fn motif_foreshadow_in_where_clause() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.query("CREATE (n:Person {id: 'a'})", &Params::new())
        .unwrap();
    e.query("CREATE (n:Person {id: 'b'})", &Params::new())
        .unwrap();

    // Every fresh insert is foreshadow=true, so the predicate matches
    // both rows.
    let r = e
        .query(
            "MATCH (n) WHERE n._motif.foreshadow = true RETURN n",
            &Params::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 2);

    // Equivalent negative: no rows currently have foreshadow=false.
    let r = e
        .query(
            "MATCH (n) WHERE n._motif.foreshadow = false RETURN n",
            &Params::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 0);
}

#[test]
fn motif_foreshadow_in_return_projection() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.query("CREATE (n:Person {id: 'a'})", &Params::new())
        .unwrap();

    let r = e
        .query("MATCH (n) RETURN n._motif.foreshadow", &Params::new())
        .unwrap();
    assert_eq!(r.columns, vec!["n._motif.foreshadow"]);
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], ResultCell::Value(Value::Bool(true)));
}

#[test]
fn unknown_motif_keys_resolve_to_null() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.query("CREATE (n:Person {id: 'a'})", &Params::new())
        .unwrap();

    // Forward-compatible probe: `_motif.something_added_in_v0_0_3` is
    // valid syntax and resolves to NULL for now.
    let r = e
        .query("MATCH (n) RETURN n._motif.future_thing", &Params::new())
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], ResultCell::Value(Value::Null));
}

#[test]
fn v0_0_1_store_is_rejected_on_open() {
    // A v0.0.1 store has `format_version = 1` in its 16-byte header.
    // v0.0.2-alpha.1 bumped FORMAT_VERSION to 2; opening a v0.0.1 file
    // surfaces a clean StorageError::BadVersion. (No migration tooling
    // by design.)
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("legacy.motif");

    // Forge a v0.0.1 header: MAGIC + version=1 + zero pad to 16 bytes.
    let mut hdr = Vec::new();
    hdr.extend_from_slice(b"MOTIF\0\0\x01"); // MAGIC
    hdr.extend_from_slice(&1u32.to_le_bytes()); // FORMAT_VERSION = 1
    hdr.extend_from_slice(&[0u8; 4]); // padding
    assert_eq!(hdr.len(), 16);
    std::fs::write(&path, hdr).unwrap();

    let opened = Engine::open(&config_with(path));
    let err = match opened {
        Ok(_) => panic!("expected v0.0.1 store to be rejected"),
        Err(e) => e,
    };
    let s = err.to_string();
    assert!(
        s.contains("unsupported format version") || s.contains("BadVersion"),
        "unexpected error: {s}"
    );
}
