//! End-to-end test for v0.0.2-alpha.3 controller-pushed schema.
//!
//! Covers MOTIF.md alpha.3 commitments:
//! - `Engine::apply_schema` persists via `MutationOp::SchemaApply`.
//! - Subsequent inserts validate labels against the current schema.
//! - Mutations against unknown labels surface a clean
//!   `EngineError::SchemaUnknown` rather than landing silently.
//! - The schema survives reopen (replays from the on-disk Mutation log).
//! - `_motif.schema.version` resolves to the current version via the
//!   metadata-as-data Cypher namespace.

use std::path::PathBuf;

use motif_core::{
    ControllerConfig, Engine, EngineError, IdentityConfig, MotifConfig, Node, Params, PropertyType,
    ResultCell, Schema, StorageConfig, TableKind, TableSchema, Value,
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
        capability: Default::default(),
        edge: Default::default(),
    }
}

fn tiny_schema(version: u64) -> Schema {
    Schema::new(version)
        .with_table(
            TableSchema::new("Person", TableKind::Node)
                .with_property("name", PropertyType::String)
                .with_property("age", PropertyType::I64),
        )
        .with_table(TableSchema::new("FOLLOWS", TableKind::Edge))
}

#[test]
fn no_schema_means_permissive() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    // No schema applied → any label is accepted.
    e.insert_node(Node::new("a", "Whatever")).unwrap();
    e.insert_node(Node::new("b", "EvenWeirder")).unwrap();
    assert!(e.current_schema().is_none());
}

#[test]
fn schema_apply_persists_in_engine() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.apply_schema(tiny_schema(1)).unwrap();
    let s = e.current_schema().unwrap();
    assert_eq!(s.version, 1);
    assert!(s.has_label("Person"));
    assert!(s.has_label("FOLLOWS"));
    assert!(!s.has_label("Robot"));
}

#[test]
fn unknown_label_rejected_after_schema_apply() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.apply_schema(tiny_schema(1)).unwrap();

    // Known label → ok.
    e.insert_node(Node::new("alice", "Person")).unwrap();

    // Unknown label → SchemaUnknown.
    let err = e.insert_node(Node::new("bot", "Robot")).unwrap_err();
    match err {
        EngineError::SchemaUnknown {
            label,
            schema_version,
        } => {
            assert_eq!(label, "Robot");
            assert_eq!(schema_version, 1);
        }
        other => panic!("expected SchemaUnknown, got {other:?}"),
    }
}

#[test]
fn schema_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("schema.db");

    {
        let mut e = Engine::open(&config_with(path.clone())).unwrap();
        e.apply_schema(tiny_schema(2)).unwrap();
        e.insert_node(Node::new("alice", "Person")).unwrap();
    }

    let reopened = Engine::open(&config_with(path)).unwrap();
    let s = reopened.current_schema().unwrap();
    assert_eq!(s.version, 2);
    assert!(s.has_label("Person"));
}

#[test]
fn newer_schema_version_supersedes() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.apply_schema(tiny_schema(1)).unwrap();
    assert_eq!(e.current_schema().unwrap().version, 1);

    // Apply v2 with a different table set.
    let v2 = Schema::new(2).with_table(TableSchema::new("Robot", TableKind::Node));
    e.apply_schema(v2).unwrap();
    let s = e.current_schema().unwrap();
    assert_eq!(s.version, 2);
    assert!(s.has_label("Robot"));
    assert!(!s.has_label("Person")); // dropped — no incremental migration
}

#[test]
fn motif_schema_version_resolves_via_cypher() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.apply_schema(tiny_schema(7)).unwrap();
    e.query("CREATE (n:Person {id: 'a'})", &Params::new())
        .unwrap();

    let r = e
        .query("MATCH (n) RETURN n._motif.schema.version", &Params::new())
        .unwrap();
    assert_eq!(r.columns, vec!["n._motif.schema.version"]);
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], ResultCell::Value(Value::I64(7)));
}

#[test]
fn motif_schema_version_is_null_when_no_schema() {
    let cfg = config_with(PathBuf::from(":memory:"));
    let mut e = Engine::open_in_memory(&cfg).unwrap();
    e.query("CREATE (n:Person {id: 'a'})", &Params::new())
        .unwrap();

    let r = e
        .query("MATCH (n) RETURN n._motif.schema.version", &Params::new())
        .unwrap();
    assert_eq!(r.rows[0][0], ResultCell::Value(Value::Null));
}
