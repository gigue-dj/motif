//! v0.0.4-alpha.3: schema property-type validation end-to-end.
//! Schema is permissive on undeclared properties; `Value::Null` is
//! accepted for any declared type (nullable-by-default).

use std::path::PathBuf;

use morceau_core::{
    ControllerConfig, Engine, EngineError, IdentityConfig, MorceauConfig, Node, PropertyType,
    Schema, StorageConfig, TableKind, TableSchema, Value,
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

fn person_schema() -> Schema {
    Schema::new(1).with_table(
        TableSchema::new("Person", TableKind::Node)
            .with_property("name", PropertyType::String)
            .with_property("age", PropertyType::I64)
            .with_property("joined_at", PropertyType::Timestamp)
            .with_property("nicknames", PropertyType::List),
    )
}

#[test]
fn insert_respecting_declared_types_succeeds() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("ok.db"));
    let mut e = Engine::open(&cfg).unwrap();
    e.apply_schema(person_schema()).unwrap();

    e.insert_node(
        Node::new("a", "Person")
            .with_property("name", "Alice")
            .with_property("age", Value::I64(30))
            .with_property("joined_at", Value::Timestamp(1_700_000_000_000))
            .with_property(
                "nicknames",
                Value::List(vec![
                    Value::String("Al".into()),
                    Value::String("Ali".into()),
                ]),
            ),
    )
    .unwrap();
}

#[test]
fn null_is_accepted_for_any_declared_type() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("null.db"));
    let mut e = Engine::open(&cfg).unwrap();
    e.apply_schema(person_schema()).unwrap();

    e.insert_node(
        Node::new("a", "Person")
            .with_property("name", Value::Null)
            .with_property("age", Value::Null)
            .with_property("joined_at", Value::Null)
            .with_property("nicknames", Value::Null),
    )
    .unwrap();
}

#[test]
fn type_mismatch_is_rejected_with_clean_error() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("mismatch.db"));
    let mut e = Engine::open(&cfg).unwrap();
    e.apply_schema(person_schema()).unwrap();

    let err = e
        .insert_node(
            Node::new("a", "Person")
                // Declared as I64, supplied F64 — no auto-widening.
                .with_property("age", Value::F64(30.5)),
        )
        .expect_err("type mismatch should be rejected");
    let EngineError::SchemaPropertyTypeMismatch {
        label,
        property,
        declared,
        actual,
        schema_version,
    } = err
    else {
        panic!("expected SchemaPropertyTypeMismatch, got {err:?}");
    };
    assert_eq!(label, "Person");
    assert_eq!(property, "age");
    assert_eq!(declared, PropertyType::I64);
    assert_eq!(actual, "f64");
    assert_eq!(schema_version, 1);
}

#[test]
fn undeclared_properties_pass_through() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("extra.db"));
    let mut e = Engine::open(&cfg).unwrap();
    e.apply_schema(person_schema()).unwrap();

    // `gpa` isn't in the schema — permissive, should succeed.
    e.insert_node(
        Node::new("a", "Person")
            .with_property("name", "Alice")
            .with_property("gpa", Value::F64(3.7)),
    )
    .unwrap();
}

#[test]
fn timestamp_and_list_round_trip_via_get_node() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("rt.db"));
    let mut e = Engine::open(&cfg).unwrap();

    let nicknames = Value::List(vec![
        Value::String("Al".into()),
        Value::String("Ali".into()),
    ]);
    e.insert_node(
        Node::new("a", "Person")
            .with_property("joined_at", Value::Timestamp(1_700_000_000_000))
            .with_property("nicknames", nicknames.clone()),
    )
    .unwrap();

    let n = e.get_node("a").unwrap().expect("inserted node");
    assert_eq!(
        n.properties.get("joined_at"),
        Some(&Value::Timestamp(1_700_000_000_000))
    );
    assert_eq!(n.properties.get("nicknames"), Some(&nicknames));
}

#[test]
fn validation_no_op_when_no_schema_set() {
    let dir = TempDir::new().unwrap();
    let cfg = config(dir.path().join("noschema.db"));
    let mut e = Engine::open(&cfg).unwrap();

    // No schema applied — anything goes (matches label-validation
    // permissive shape).
    e.insert_node(
        Node::new("a", "Person")
            .with_property("age", Value::F64(30.5))
            .with_property("nicknames", Value::Bool(true)),
    )
    .unwrap();
}

#[test]
fn schema_race_recovery_replays_old_records_under_newer_schema() {
    // v0.0.2 exit criterion 5: schema race surfaces a clean state,
    // not silent corruption. Scenario: insert under schema v1 →
    // apply schema v2 that drops the `Person` label → reopen.
    // Recovery must replay the old `Person` insert without crashing
    // (recovery is permissive — it's reconstructing committed state,
    // not re-validating). New inserts of `Person` after recovery
    // are correctly rejected by the latest schema.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("race.db");

    {
        let cfg = config(path.clone());
        let mut e = Engine::open(&cfg).unwrap();
        e.apply_schema(Schema::new(1).with_table(TableSchema::new("Person", TableKind::Node)))
            .unwrap();
        e.insert_node(Node::new("a", "Person")).unwrap();
        // Schema v2 forgets about `Person`.
        e.apply_schema(Schema::new(2).with_table(TableSchema::new("Robot", TableKind::Node)))
            .unwrap();
    }

    let cfg = config(path);
    let mut e = Engine::open(&cfg).unwrap();
    // Old `Person` record survives recovery.
    let n = e.get_node("a").unwrap().expect("Person 'a' replayed");
    assert_eq!(n.label, "Person");
    // Latest schema is v2 (the second apply_schema).
    let schema = e.current_schema().expect("schema present after recovery");
    assert_eq!(schema.version, 2);
    // New `Person` insert IS rejected — current schema is v2 and
    // doesn't know `Person`.
    let err = e
        .insert_node(Node::new("b", "Person"))
        .expect_err("insert with dropped label should fail");
    assert!(matches!(err, EngineError::SchemaUnknown { .. }), "{err:?}");
}
