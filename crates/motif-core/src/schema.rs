//! Schema types pushed by the controller. v0.0.2-alpha.3 introduces
//! controller-owned schemas: the controller pushes a [`Schema`] over
//! the same channel as mutations, motif persists it to the on-disk
//! Mutation log via [`MutationOp::SchemaApply`], and subsequent
//! mutations are validated against the latest schema (unknown labels
//! surface a clean `EngineError::SchemaUnknown` rather than landing
//! silently).
//!
//! v0.0.2-alpha.3 keeps schemas intentionally thin: they enumerate the
//! known node and edge labels and the property names + scalar types
//! each label may carry. Property-type validation, optional/required
//! markings, and migration semantics are post-v0.0.2 work.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A controller-pushed schema. The `version` is monotonic; later
/// versions supersede earlier ones in their entirety (no partial /
/// incremental updates in v0.0.2-alpha.3 — those land with migration
/// in v0.0.3+).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    pub version: u64,
    pub tables: BTreeMap<String, TableSchema>,
}

impl Schema {
    /// Fluent constructor for tests / CLI demos.
    pub fn new(version: u64) -> Self {
        Self {
            version,
            tables: BTreeMap::new(),
        }
    }

    pub fn with_table(mut self, t: TableSchema) -> Self {
        self.tables.insert(t.label.clone(), t);
        self
    }

    /// True iff the schema knows a label.
    pub fn has_label(&self, label: &str) -> bool {
        self.tables.contains_key(label)
    }

    /// Look up the table for a label.
    pub fn table(&self, label: &str) -> Option<&TableSchema> {
        self.tables.get(label)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableSchema {
    pub label: String,
    pub kind: TableKind,
    #[serde(default)]
    pub properties: BTreeMap<String, PropertyType>,
}

impl TableSchema {
    pub fn new(label: impl Into<String>, kind: TableKind) -> Self {
        Self {
            label: label.into(),
            kind,
            properties: BTreeMap::new(),
        }
    }

    pub fn with_property(mut self, key: impl Into<String>, ty: PropertyType) -> Self {
        self.properties.insert(key.into(), ty);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TableKind {
    Node,
    Edge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PropertyType {
    Bool,
    I64,
    F64,
    String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluent_construction() {
        let s = Schema::new(1).with_table(
            TableSchema::new("Person", TableKind::Node)
                .with_property("name", PropertyType::String)
                .with_property("age", PropertyType::I64),
        );
        assert!(s.has_label("Person"));
        assert!(!s.has_label("Robot"));
        let t = s.table("Person").unwrap();
        assert_eq!(t.kind, TableKind::Node);
        assert_eq!(t.properties.len(), 2);
    }

    #[test]
    fn schema_round_trips_via_bincode() {
        let s = Schema::new(7).with_table(
            TableSchema::new("Edge", TableKind::Edge).with_property("weight", PropertyType::F64),
        );
        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(&s, cfg).unwrap();
        let (back, _): (Schema, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
        assert_eq!(back, s);
    }
}
