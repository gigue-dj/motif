//! End-to-end smoke test for the alpha.3 storage engine: durable append +
//! recovery across reopen. This is the closest thing v0.0.1 has to an
//! "exit criterion 2" test (`Motif::open(&MotifConfig)` works end-to-end)
//! while the wasm-bindgen layer is still pending.

use std::path::PathBuf;

use motif_core::{
    ControllerConfig, Edge, Engine, IdentityConfig, MotifConfig, Node, StorageConfig, Value,
};
use tempfile::TempDir;

fn config_with(path: PathBuf) -> MotifConfig {
    MotifConfig {
        identity: IdentityConfig {
            user_id: "u_abc".into(),
            device_id: "d_xyz".into(),
        },
        controller: ControllerConfig {
            kind: "in-memory".into(),
        },
        storage: StorageConfig { path },
        capability: Default::default(),
        edge: Default::default(),
    }
}

#[test]
fn durable_append_recovers_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("motif.db");

    {
        let mut e = Engine::open(&config_with(path.clone())).unwrap();
        e.insert_node(Node::new("alice", "Person").with_property("age", Value::I64(30)))
            .unwrap();
        e.insert_node(Node::new("bob", "Person").with_property("age", Value::I64(31)))
            .unwrap();
        e.insert_edge(
            Edge::new("a_follows_b", "FOLLOWS", "alice", "bob")
                .with_property("since", Value::I64(2026)),
        )
        .unwrap();
        assert_eq!(e.node_count(), 2);
        assert_eq!(e.edge_count(), 1);
    }

    let mut reopened = Engine::open(&config_with(path)).unwrap();
    assert_eq!(reopened.node_count(), 2);
    assert_eq!(reopened.edge_count(), 1);

    let alice = reopened.get_node("alice").unwrap().unwrap();
    assert_eq!(alice.label, "Person");
    assert_eq!(alice.properties["age"], Value::I64(30));

    let edge = reopened.get_edge("a_follows_b").unwrap().unwrap();
    assert_eq!(edge.from, "alice");
    assert_eq!(edge.to, "bob");
    assert_eq!(edge.properties["since"], Value::I64(2026));
}

#[test]
fn torn_tail_is_truncated_on_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("torn.db");

    {
        let mut e = Engine::open(&config_with(path.clone())).unwrap();
        e.insert_node(Node::new("a", "Person")).unwrap();
        e.insert_node(Node::new("b", "Person")).unwrap();
    }

    // Simulate a torn write: append a length prefix that promises more
    // bytes than exist on disk.
    {
        use std::fs::OpenOptions;
        use std::io::Write;
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&999u32.to_le_bytes()).unwrap();
        f.write_all(b"oops").unwrap();
    }

    let mut reopened = Engine::open(&config_with(path.clone())).unwrap();
    // The two committed nodes should still be there; the torn tail is gone.
    assert_eq!(reopened.node_count(), 2);
    assert!(reopened.get_node("a").unwrap().is_some());
    assert!(reopened.get_node("b").unwrap().is_some());

    // And we should be able to keep writing after recovery.
    reopened.insert_node(Node::new("c", "Person")).unwrap();
    assert_eq!(reopened.node_count(), 3);
}
