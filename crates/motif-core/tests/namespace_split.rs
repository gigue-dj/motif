//! v0.0.4-alpha.1: id-namespace split + edge-by-label index.
//!
//! Tests the two new invariants:
//! 1. A node and an edge can share the same id without colliding —
//!    the typed accessors (`get_node` / `get_edge`) keep the
//!    namespaces unambiguous. Per the answer to the v0.0.4-alpha.1
//!    "duplication / idempotency" concern: the split changes the
//!    index shape only, not record cardinality. A graph with N nodes
//!    + M edges has N + M index entries, never more.
//! 2. `iter_edges_by_label` is an O(1) lookup into `edge_by_label`,
//!    and its invariants under insert / delete (and label-bucket
//!    cleanup when emptied) hold.

use std::path::PathBuf;

use motif_core::{
    ControllerConfig, Edge, Engine, IdentityConfig, MotifConfig, Node, StorageConfig, Value,
};
use tempfile::TempDir;

fn config(path: PathBuf) -> MotifConfig {
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

#[test]
fn node_and_edge_can_share_an_id() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("split.db"));
    let mut e = Engine::open(&cfg).unwrap();

    // Insert a node "x" and two anchor nodes for the edge.
    e.insert_node(Node::new("x", "Person")).unwrap();
    e.insert_node(Node::new("a", "Person")).unwrap();
    e.insert_node(Node::new("b", "Person")).unwrap();

    // Edge "x" coexists with node "x" — no DuplicateId — and lives in
    // its own namespace.
    e.insert_edge(Edge::new("x", "KNOWS", "a", "b")).unwrap();

    let node = e.get_node("x").unwrap().expect("node x");
    let edge = e.get_edge("x").unwrap().expect("edge x");
    assert_eq!(node.label, "Person");
    assert_eq!(edge.label, "KNOWS");
    assert!(e.has_node("x"));
    assert!(e.has_edge("x"));
    assert_eq!(e.node_count(), 3);
    assert_eq!(e.edge_count(), 1);
}

#[test]
fn node_dup_still_errors_within_node_namespace() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("node_dup.db"));
    let mut e = Engine::open(&cfg).unwrap();

    e.insert_node(Node::new("a", "Person")).unwrap();
    let err = e
        .insert_node(Node::new("a", "Person"))
        .expect_err("duplicate node id should still fail");
    let msg = format!("{err}");
    assert!(msg.to_lowercase().contains("duplicate"), "{msg}");
}

#[test]
fn edge_dup_still_errors_within_edge_namespace() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("edge_dup.db"));
    let mut e = Engine::open(&cfg).unwrap();

    e.insert_node(Node::new("a", "Person")).unwrap();
    e.insert_node(Node::new("b", "Person")).unwrap();
    e.insert_edge(Edge::new("r1", "KNOWS", "a", "b")).unwrap();
    let err = e
        .insert_edge(Edge::new("r1", "KNOWS", "a", "b"))
        .expect_err("duplicate edge id should still fail");
    let msg = format!("{err}");
    assert!(msg.to_lowercase().contains("duplicate"), "{msg}");
}

#[test]
fn iter_edges_by_label_returns_only_matching_edges() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("by_label.db"));
    let mut e = Engine::open(&cfg).unwrap();

    e.insert_node(Node::new("a", "Person")).unwrap();
    e.insert_node(Node::new("b", "Person")).unwrap();
    e.insert_edge(Edge::new("r1", "KNOWS", "a", "b")).unwrap();
    e.insert_edge(Edge::new("r2", "KNOWS", "b", "a")).unwrap();
    e.insert_edge(Edge::new("r3", "FOLLOWS", "a", "b")).unwrap();

    let knows = e.iter_edges_by_label("KNOWS").unwrap();
    assert_eq!(knows.len(), 2);
    let mut ids: Vec<&str> = knows.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["r1", "r2"]);

    let follows = e.iter_edges_by_label("FOLLOWS").unwrap();
    assert_eq!(follows.len(), 1);
    assert_eq!(follows[0].id, "r3");

    let missing = e.iter_edges_by_label("BLOCKS").unwrap();
    assert!(missing.is_empty());
}

#[test]
fn iter_edges_by_label_reflects_deletes() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("delete.db"));
    let mut e = Engine::open(&cfg).unwrap();

    e.insert_node(Node::new("a", "Person")).unwrap();
    e.insert_node(Node::new("b", "Person")).unwrap();
    e.insert_edge(Edge::new("r1", "KNOWS", "a", "b")).unwrap();
    e.insert_edge(Edge::new("r2", "KNOWS", "b", "a")).unwrap();

    assert_eq!(e.iter_edges_by_label("KNOWS").unwrap().len(), 2);

    e.delete_edge("r1").unwrap();
    let remaining = e.iter_edges_by_label("KNOWS").unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "r2");

    e.delete_edge("r2").unwrap();
    // Bucket should be empty AND removed — verified by absence at
    // iter time (returns empty Vec for missing labels).
    assert!(e.iter_edges_by_label("KNOWS").unwrap().is_empty());
}

#[test]
fn iter_edges_by_label_survives_reopen() {
    // The edge_by_label index lives in memory; reopen has to
    // rebuild it from the on-disk log via `apply_recovered`.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reopen.db");

    {
        let cfg = config(path.clone());
        let mut e = Engine::open(&cfg).unwrap();
        e.insert_node(Node::new("a", "Person")).unwrap();
        e.insert_node(Node::new("b", "Person")).unwrap();
        e.insert_edge(Edge::new("r1", "KNOWS", "a", "b")).unwrap();
        e.insert_edge(Edge::new("r2", "FOLLOWS", "b", "a")).unwrap();
        e.insert_edge(Edge::new("r3", "KNOWS", "a", "b").with_property("since", Value::I64(2020)))
            .unwrap();
        e.delete_edge("r1").unwrap();
    }

    let cfg = config(path);
    let mut e = Engine::open(&cfg).unwrap();
    let knows = e.iter_edges_by_label("KNOWS").unwrap();
    assert_eq!(knows.len(), 1);
    assert_eq!(knows[0].id, "r3");

    let follows = e.iter_edges_by_label("FOLLOWS").unwrap();
    assert_eq!(follows.len(), 1);
    assert_eq!(follows[0].id, "r2");
}
