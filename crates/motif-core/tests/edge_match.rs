//! v0.0.4-alpha.2: Cypher edge surface integration tests.
//!
//! Covers:
//! - `MATCH (a)-[r]->(b) RETURN r` — basic edge binding.
//! - `MATCH (a)-[r:LABEL]->(b)` — edge label predicate via `edge_by_label`.
//! - `MATCH (a)-[r:LABEL {since: 2020}]->(b)` — inline property
//!   predicates on the edge pattern (bucket-scan filter; a dedicated
//!   edge-property index is deferred to alpha.3+).
//! - `MATCH (a)-[r]->(b), (b)-[s]->(c)` — multi-pattern with shared
//!   variable enforcement.
//! - Multi-hop paths `MATCH (a)-[r]->(b)-[s]->(c)`.
//! - `RETURN r.prop` on the edge binding.

use std::path::PathBuf;

use motif_core::{
    ControllerConfig, Edge, Engine, IdentityConfig, MotifConfig, Node, Params, ResultCell,
    StorageConfig, Value,
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

fn seed(engine: &mut Engine) {
    engine.insert_node(Node::new("a", "Person")).unwrap();
    engine.insert_node(Node::new("b", "Person")).unwrap();
    engine.insert_node(Node::new("c", "Person")).unwrap();
    engine
        .insert_edge(Edge::new("r1", "KNOWS", "a", "b").with_property("since", Value::I64(2020)))
        .unwrap();
    engine
        .insert_edge(Edge::new("r2", "KNOWS", "b", "c").with_property("since", Value::I64(2024)))
        .unwrap();
    engine
        .insert_edge(Edge::new("r3", "FOLLOWS", "a", "c"))
        .unwrap();
}

#[test]
fn single_edge_pattern_returns_all_edges() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("edges.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let r = e
        .query("MATCH (a)-[r]->(b) RETURN r", &Params::new())
        .unwrap();
    assert_eq!(r.rows.len(), 3);
    let mut ids: Vec<String> = r
        .rows
        .iter()
        .map(|row| match &row[0] {
            ResultCell::Edge(e) => e.id.clone(),
            _ => panic!("expected Edge cell"),
        })
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["r1", "r2", "r3"]);
}

#[test]
fn edge_pattern_label_predicate_uses_label_index() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("label.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let r = e
        .query("MATCH (a)-[r:KNOWS]->(b) RETURN r", &Params::new())
        .unwrap();
    assert_eq!(r.rows.len(), 2);
    let mut ids: Vec<String> = r
        .rows
        .iter()
        .map(|row| match &row[0] {
            ResultCell::Edge(e) => e.id.clone(),
            _ => panic!(),
        })
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["r1", "r2"]);
}

#[test]
fn edge_inline_property_predicate_filters() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("inline.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let r = e
        .query(
            "MATCH (a)-[r:KNOWS {since: 2020}]->(b) RETURN r",
            &Params::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    let ResultCell::Edge(edge) = &r.rows[0][0] else {
        panic!()
    };
    assert_eq!(edge.id, "r1");
}

#[test]
fn return_edge_property_works() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("rprop.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let r = e
        .query("MATCH (a)-[r:KNOWS]->(b) RETURN r.since", &Params::new())
        .unwrap();
    assert_eq!(r.rows.len(), 2);
    let mut years: Vec<i64> = r
        .rows
        .iter()
        .map(|row| match &row[0] {
            ResultCell::Value(Value::I64(n)) => *n,
            _ => panic!("expected I64 cell"),
        })
        .collect();
    years.sort();
    assert_eq!(years, vec![2020, 2024]);
}

#[test]
fn multi_hop_path_chains_bindings() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("multihop.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let r = e
        .query(
            "MATCH (a)-[r:KNOWS]->(b)-[s:KNOWS]->(c) RETURN a, c",
            &Params::new(),
        )
        .unwrap();
    // Only (a)-r1->(b)-r2->(c) satisfies KNOWS-KNOWS.
    assert_eq!(r.rows.len(), 1);
    let (ResultCell::Node(start), ResultCell::Node(end)) = (&r.rows[0][0], &r.rows[0][1]) else {
        panic!()
    };
    assert_eq!(start.id, "a");
    assert_eq!(end.id, "c");
}

#[test]
fn multi_pattern_unifies_on_shared_variable() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("multipat.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let r = e
        .query(
            "MATCH (a)-[r:KNOWS]->(b), (b)-[s:KNOWS]->(c) RETURN a, c",
            &Params::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    let (ResultCell::Node(start), ResultCell::Node(end)) = (&r.rows[0][0], &r.rows[0][1]) else {
        panic!()
    };
    assert_eq!(start.id, "a");
    assert_eq!(end.id, "c");
}

#[test]
fn multi_pattern_with_disjoint_variables_returns_cartesian_product() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("disjoint.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    // No shared variables → no unification constraint → cartesian
    // product. First pattern matches r1, r2 (both KNOWS); second
    // pattern (independent vars) matches r1, r2 again. 2 × 2 = 4.
    let r = e
        .query(
            "MATCH (a)-[r:KNOWS]->(b), (c)-[s:KNOWS]->(d) RETURN a",
            &Params::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 4);
}

#[test]
fn unlabeled_edge_pattern_walks_iter_edges() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("unlabeled.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let r = e
        .query("MATCH (a)-[r {since: 2024}]->(b) RETURN r", &Params::new())
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    let ResultCell::Edge(edge) = &r.rows[0][0] else {
        panic!()
    };
    assert_eq!(edge.id, "r2");
}

#[test]
fn edge_pattern_label_with_zero_matches_returns_empty() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("zero.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed(&mut e);

    let r = e
        .query("MATCH (a)-[r:BLOCKS]->(b) RETURN r", &Params::new())
        .unwrap();
    assert!(r.rows.is_empty());
}
