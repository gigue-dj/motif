//! End-to-end query test: exercises CREATE, MATCH/WHERE/RETURN, MERGE,
//! and MATCH/DELETE through `Engine::query`. This is the closest thing
//! v0.0.1 has to the "Motif::query works end-to-end" exit criterion
//! while the wasm-bindgen layer is still pending.

use std::path::PathBuf;

use motif_core::{
    ControllerConfig, Engine, IdentityConfig, MotifConfig, Params, ResultCell, StorageConfig, Value,
};
use tempfile::TempDir;

fn config_with(path: PathBuf) -> MotifConfig {
    MotifConfig {
        identity: IdentityConfig {
            user_id: "u".into(),
            device_id: "d".into(),
        },
        controller: ControllerConfig {
            kind: "in-memory".into(),
        },
        storage: StorageConfig { path },
    }
}

fn engine() -> (TempDir, Engine) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("q.db");
    let engine = Engine::open(&config_with(path)).unwrap();
    (dir, engine)
}

#[test]
fn create_and_match_by_id_round_trips() {
    let (_dir, mut e) = engine();

    e.query(
        "CREATE (n:Person {id: 'alice', name: 'Alice', age: 30})",
        &Params::new(),
    )
    .unwrap();

    let mut params = Params::new();
    params.insert("x".into(), Value::String("alice".into()));
    let r = e
        .query("MATCH (n) WHERE id(n) = $x RETURN n.name, n.age", &params)
        .unwrap();

    assert_eq!(r.columns, vec!["n.name", "n.age"]);
    assert_eq!(r.rows.len(), 1);
    assert_eq!(
        r.rows[0][0],
        ResultCell::Value(Value::String("Alice".into()))
    );
    assert_eq!(r.rows[0][1], ResultCell::Value(Value::I64(30)));
}

#[test]
fn match_with_label_filter_and_limit() {
    let (_dir, mut e) = engine();

    for (id, label, age) in [
        ("a", "Person", 20),
        ("b", "Person", 25),
        ("c", "Person", 40),
        ("d", "Bot", 0),
    ] {
        let mut p = Params::new();
        p.insert("id".into(), Value::String(id.into()));
        p.insert("age".into(), Value::I64(age));
        let q = format!("CREATE (n:{label} {{id: $id, age: $age}})");
        e.query(&q, &p).unwrap();
    }

    // Label filter + WHERE comparison + limit.
    let r = e
        .query(
            "MATCH (n:Person) WHERE n.age >= 25 RETURN n LIMIT 5",
            &Params::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 2);

    for row in &r.rows {
        match &row[0] {
            ResultCell::Node(n) => assert_eq!(n.label, "Person"),
            _ => panic!("expected node"),
        }
    }
}

#[test]
fn merge_is_idempotent() {
    let (_dir, mut e) = engine();

    e.query("MERGE (n:Person {id: 'p1', name: 'Alice'})", &Params::new())
        .unwrap();
    // Second MERGE with the same id is a no-op (does NOT update name).
    e.query(
        "MERGE (n:Person {id: 'p1', name: 'Alicia'})",
        &Params::new(),
    )
    .unwrap();

    assert_eq!(e.node_count(), 1);
    let n = e.get_node("p1").unwrap().unwrap();
    assert_eq!(n.properties["name"], Value::String("Alice".into()));
}

#[test]
fn match_delete_removes_node() {
    let (_dir, mut e) = engine();

    e.query("CREATE (n:Person {id: 'doomed'})", &Params::new())
        .unwrap();
    assert_eq!(e.node_count(), 1);

    let mut params = Params::new();
    params.insert("x".into(), Value::String("doomed".into()));
    e.query("MATCH (n) WHERE id(n) = $x DELETE n", &params)
        .unwrap();
    assert_eq!(e.node_count(), 0);
    assert!(e.get_node("doomed").unwrap().is_none());
}

#[test]
fn delete_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("d.db");

    {
        let mut e = Engine::open(&config_with(path.clone())).unwrap();
        e.query("CREATE (n:Person {id: 'keep'})", &Params::new())
            .unwrap();
        e.query("CREATE (n:Person {id: 'gone'})", &Params::new())
            .unwrap();
        let mut p = Params::new();
        p.insert("x".into(), Value::String("gone".into()));
        e.query("MATCH (n) WHERE id(n) = $x DELETE n", &p).unwrap();
    }

    let mut reopened = Engine::open(&config_with(path)).unwrap();
    assert_eq!(reopened.node_count(), 1);
    assert!(reopened.get_node("keep").unwrap().is_some());
    assert!(reopened.get_node("gone").unwrap().is_none());
}

#[test]
fn parse_errors_surface_via_query() {
    let (_dir, mut e) = engine();
    let err = e.query("CREATE 42", &Params::new()).unwrap_err();
    assert!(err.to_string().contains("query"));
}
