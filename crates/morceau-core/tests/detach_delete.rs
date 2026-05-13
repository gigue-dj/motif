//! v0.0.4-alpha.3: end-to-end coverage for `DETACH DELETE` cascade
//! semantics.
//!
//! Covers:
//! - `DETACH DELETE n` removes the node + every incident edge
//!   (`from == n.id || to == n.id`).
//! - Plain `DELETE n` (no `DETACH`) leaves edges dangling — same
//!   behavior as before alpha.3.
//! - `Engine::delete_node_with_cascade` returns the count of edges
//!   removed and `(false, 0)` when the node doesn't exist.
//! - Cascade commits edges first, then the node — recovery sees
//!   the consistent state.

use std::path::PathBuf;

use morceau_core::{
    ControllerConfig, Edge, Engine, IdentityConfig, MorceauConfig, Node, Params, StorageConfig,
};
use tempfile::TempDir;

fn config(path: PathBuf) -> MorceauConfig {
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

fn seed(engine: &mut Engine) {
    engine.insert_node(Node::new("a", "Person")).unwrap();
    engine.insert_node(Node::new("b", "Person")).unwrap();
    engine.insert_node(Node::new("c", "Person")).unwrap();
    // a → b, a → c, b → a — all incident on 'a'.
    engine
        .insert_edge(Edge::new("r1", "KNOWS", "a", "b"))
        .unwrap();
    engine
        .insert_edge(Edge::new("r2", "KNOWS", "a", "c"))
        .unwrap();
    engine
        .insert_edge(Edge::new("r3", "FOLLOWS", "b", "a"))
        .unwrap();
    // b → c — NOT incident on 'a'; should survive a DETACH DELETE of 'a'.
    engine
        .insert_edge(Edge::new("r4", "KNOWS", "b", "c"))
        .unwrap();
}

#[test]
fn detach_delete_removes_node_and_incident_edges() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("detach.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    e.query(
        "MATCH (n) WHERE id(n) = 'a' DETACH DELETE n",
        &Params::new(),
    )
    .unwrap();

    assert!(!e.has_node("a"));
    // Incident edges (r1, r2, r3) are gone; the b→c edge (r4) survives.
    assert!(e.get_edge("r1").unwrap().is_none());
    assert!(e.get_edge("r2").unwrap().is_none());
    assert!(e.get_edge("r3").unwrap().is_none());
    assert!(e.get_edge("r4").unwrap().is_some());
}

#[test]
fn plain_delete_leaves_edges_dangling() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("plain.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    e.query("MATCH (n) WHERE id(n) = 'a' DELETE n", &Params::new())
        .unwrap();

    assert!(!e.has_node("a"));
    // No cascade — incident edges still exist (and their from/to
    // pointers now reference a missing node, per the documented
    // engine behavior).
    assert!(e.get_edge("r1").unwrap().is_some());
    assert!(e.get_edge("r2").unwrap().is_some());
    assert!(e.get_edge("r3").unwrap().is_some());
}

#[test]
fn delete_node_with_cascade_reports_count() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("count.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let (existed, removed) = e.delete_node_with_cascade("a").unwrap();
    assert!(existed);
    assert_eq!(removed, 3);
    // r4 is the only KNOWS edge left.
    assert_eq!(e.iter_edges_by_label("KNOWS").unwrap().len(), 1);
}

#[test]
fn delete_node_with_cascade_returns_false_for_missing_node() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("missing.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let (existed, removed) = e.delete_node_with_cascade("nope").unwrap();
    assert!(!existed);
    assert_eq!(removed, 0);
    // Sanity: nothing else changed.
    assert_eq!(e.node_count(), 3);
    assert_eq!(e.edge_count(), 4);
}

#[test]
fn detach_delete_survives_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reopen.db");

    {
        let cfg = config(path.clone());
        let mut e = Engine::open(&cfg).unwrap();
        seed(&mut e);
        e.query(
            "MATCH (n) WHERE id(n) = 'a' DETACH DELETE n",
            &Params::new(),
        )
        .unwrap();
    }

    let cfg = config(path);
    let mut e = Engine::open(&cfg).unwrap();
    assert!(!e.has_node("a"));
    assert!(e.get_edge("r1").unwrap().is_none());
    assert!(e.get_edge("r2").unwrap().is_none());
    assert!(e.get_edge("r3").unwrap().is_none());
    assert!(e.get_edge("r4").unwrap().is_some());
}
