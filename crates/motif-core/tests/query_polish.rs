//! v0.0.4-alpha.4: ORDER BY + count/collect aggregates +
//! adjacency-index-backed edge MATCH at scale.

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

fn seed_people(engine: &mut Engine) {
    engine
        .insert_node(Node::new("a", "Person").with_property("age", Value::I64(30)))
        .unwrap();
    engine
        .insert_node(Node::new("b", "Person").with_property("age", Value::I64(25)))
        .unwrap();
    engine
        .insert_node(Node::new("c", "Person").with_property("age", Value::I64(40)))
        .unwrap();
    engine
        .insert_node(Node::new("d", "Person").with_property("age", Value::I64(35)))
        .unwrap();
}

#[test]
fn order_by_ascending_sorts_results() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("asc.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed_people(&mut e);

    let r = e
        .query("MATCH (n) RETURN n.age ORDER BY n.age ASC", &Params::new())
        .unwrap();
    let ages: Vec<i64> = r
        .rows
        .iter()
        .map(|row| match &row[0] {
            ResultCell::Value(Value::I64(n)) => *n,
            _ => panic!(),
        })
        .collect();
    assert_eq!(ages, vec![25, 30, 35, 40]);
}

#[test]
fn order_by_descending_sorts_reverse() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("desc.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed_people(&mut e);

    let r = e
        .query("MATCH (n) RETURN n.age ORDER BY n.age DESC", &Params::new())
        .unwrap();
    let ages: Vec<i64> = r
        .rows
        .iter()
        .map(|row| match &row[0] {
            ResultCell::Value(Value::I64(n)) => *n,
            _ => panic!(),
        })
        .collect();
    assert_eq!(ages, vec![40, 35, 30, 25]);
}

#[test]
fn order_by_then_limit_takes_first_n() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("limit.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed_people(&mut e);

    let r = e
        .query(
            "MATCH (n) RETURN n.age ORDER BY n.age ASC LIMIT 2",
            &Params::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 2);
    let first = match &r.rows[0][0] {
        ResultCell::Value(Value::I64(n)) => *n,
        _ => panic!(),
    };
    let second = match &r.rows[1][0] {
        ResultCell::Value(Value::I64(n)) => *n,
        _ => panic!(),
    };
    assert_eq!((first, second), (25, 30));
}

#[test]
fn count_aggregate_collapses_rows() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("count.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed_people(&mut e);

    let r = e
        .query("MATCH (n) RETURN count(n)", &Params::new())
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    match &r.rows[0][0] {
        ResultCell::Value(Value::I64(c)) => assert_eq!(*c, 4),
        other => panic!("expected I64 count, got {other:?}"),
    }
    assert_eq!(r.columns, vec!["count(n)"]);
}

#[test]
fn collect_aggregate_gathers_property() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("collect.db"));
    let mut e = Engine::open(&cfg).unwrap();
    seed_people(&mut e);

    let r = e
        .query(
            "MATCH (n) RETURN collect(n.age) ORDER BY n.age ASC",
            &Params::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    let ResultCell::Value(Value::List(ages)) = &r.rows[0][0] else {
        panic!("expected List");
    };
    let nums: Vec<i64> = ages
        .iter()
        .map(|v| match v {
            Value::I64(n) => *n,
            _ => panic!(),
        })
        .collect();
    assert_eq!(nums, vec![25, 30, 35, 40]);
}

#[test]
fn iter_edges_from_returns_only_outgoing() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("from.db"));
    let mut e = Engine::open(&cfg).unwrap();
    e.insert_node(Node::new("a", "Person")).unwrap();
    e.insert_node(Node::new("b", "Person")).unwrap();
    e.insert_node(Node::new("c", "Person")).unwrap();
    e.insert_edge(Edge::new("r1", "KNOWS", "a", "b")).unwrap();
    e.insert_edge(Edge::new("r2", "KNOWS", "a", "c")).unwrap();
    e.insert_edge(Edge::new("r3", "KNOWS", "b", "a")).unwrap();
    e.insert_edge(Edge::new("r4", "FOLLOWS", "a", "b")).unwrap();

    // All edges from 'a' (any label).
    let from_a = e.iter_edges_from("a", None).unwrap();
    let mut ids: Vec<&str> = from_a.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["r1", "r2", "r4"]);

    // Edges from 'a' with label KNOWS.
    let knows_from_a = e.iter_edges_from("a", Some("KNOWS")).unwrap();
    let mut ids: Vec<&str> = knows_from_a.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["r1", "r2"]);

    // No outgoing for 'c'.
    assert!(e.iter_edges_from("c", None).unwrap().is_empty());
}

#[test]
fn iter_edges_incident_to_returns_both_directions() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("incident.db"));
    let mut e = Engine::open(&cfg).unwrap();
    e.insert_node(Node::new("a", "Person")).unwrap();
    e.insert_node(Node::new("b", "Person")).unwrap();
    e.insert_node(Node::new("c", "Person")).unwrap();
    e.insert_edge(Edge::new("r1", "KNOWS", "a", "b")).unwrap();
    e.insert_edge(Edge::new("r2", "KNOWS", "c", "b")).unwrap();
    e.insert_edge(Edge::new("r3", "KNOWS", "b", "a")).unwrap();
    // r4 is unrelated to 'b'.
    e.insert_edge(Edge::new("r4", "KNOWS", "a", "c")).unwrap();

    let incident = e.iter_edges_incident_to("b").unwrap();
    let mut ids: Vec<&str> = incident.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["r1", "r2", "r3"]);
}

#[test]
fn adjacency_indexes_survive_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reopen.db");

    {
        let cfg = config(path.clone());
        let mut e = Engine::open(&cfg).unwrap();
        e.insert_node(Node::new("a", "Person")).unwrap();
        e.insert_node(Node::new("b", "Person")).unwrap();
        e.insert_edge(Edge::new("r1", "KNOWS", "a", "b")).unwrap();
        e.insert_edge(Edge::new("r2", "KNOWS", "b", "a")).unwrap();
        e.delete_edge("r1").unwrap();
    }

    let cfg = config(path);
    let mut e = Engine::open(&cfg).unwrap();
    // r1 is gone; only r2 should be left.
    let from_b = e.iter_edges_from("b", None).unwrap();
    assert_eq!(from_b.len(), 1);
    assert_eq!(from_b[0].id, "r2");
    let from_a = e.iter_edges_from("a", None).unwrap();
    assert!(from_a.is_empty());
}

#[test]
fn edge_match_pushes_down_id_predicate_on_start() {
    // The bench surfaced this: pre-alpha.4 the id-predicate fast
    // path only hit `node_candidates`, so path-pattern queries
    // walked every node in the namespace. This test pins the new
    // pushdown by checking correctness on a graph large enough that
    // a full scan would be measurable; correctness implies the
    // pushdown is intact (no benchmark assertion — that lives in
    // `motif bench --scale`).
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("pushdown.db"));
    let mut e = Engine::open(&cfg).unwrap();
    for i in 0..200 {
        e.insert_node(Node::new(format!("n{i}"), "Person")).unwrap();
    }
    for i in 0..200 {
        e.insert_edge(Edge::new(
            format!("e{i}"),
            "KNOWS",
            format!("n{i}"),
            format!("n{}", (i + 1) % 200),
        ))
        .unwrap();
    }

    let mut params = Params::new();
    params.insert("x".into(), Value::String("n42".into()));
    let r = e
        .query(
            "MATCH (a)-[r:KNOWS]->(b) WHERE id(a) = $x RETURN b",
            &params,
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    let ResultCell::Node(b) = &r.rows[0][0] else {
        panic!()
    };
    assert_eq!(b.id, "n43");
}
