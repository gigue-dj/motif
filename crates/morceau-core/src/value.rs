//! Scalar values stored as node and edge properties.
//!
//! ## Variants
//!
//! v0.0.4-alpha.3 ships seven variants:
//!
//! - `Null`           — sentinel for "no value" / missing optional.
//! - `Bool(bool)`     — true / false.
//! - `I64(i64)`       — signed 64-bit integer.
//! - `F64(f64)`       — IEEE 754 double.
//! - `String(String)` — UTF-8 text.
//! - `Timestamp(i64)` — unix epoch milliseconds, signed (pre-1970 + post-2262 representable).
//! - `List(Vec<Value>)` — heterogeneous list.
//!
//! ## Codec layout (on-disk discriminants)
//!
//! Variants are serialized via `serde` + `bincode` 2; the discriminant
//! is the variant's index in source order. **The order in this file is
//! the source of truth for the on-disk format.** Reordering — or
//! inserting a variant before the last one — silently breaks every
//! store on the planet. Indices 0–6 (`Null` … `List`) are shipped.
//! New variants append at the end and bump
//! `storage::FORMAT_VERSION`. The
//! `bincode_round_trip_preserves_discriminant_order` test fingerprints
//! 0–6 so any reorder fails loudly.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Value {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    /// Unix epoch milliseconds. Signed so dates pre-1970 (and post
    /// the i32-seconds rollover in 2038, and post 2262 when
    /// nanoseconds exceed i64) round-trip without loss.
    Timestamp(i64),
    /// Heterogeneous list. Element types aren't constrained at the
    /// `Value` layer; the schema's `PropertyType::List` variant is
    /// likewise untyped in v0.0.4-alpha.3 — typed lists
    /// (`PropertyType::List(Box<PropertyType>)`) wait for a caller
    /// that needs them.
    List(Vec<Value>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::I64(_) => "i64",
            Value::F64(_) => "f64",
            Value::String(_) => "string",
            Value::Timestamp(_) => "timestamp",
            Value::List(_) => "list",
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::I64(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::F64(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_owned())
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Value::List(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_name_covers_every_variant() {
        let samples = [
            Value::Null,
            Value::Bool(true),
            Value::I64(0),
            Value::F64(0.0),
            Value::String(String::new()),
            Value::Timestamp(0),
            Value::List(vec![]),
        ];
        let names: Vec<&str> = samples.iter().map(|v| v.type_name()).collect();
        assert_eq!(
            names,
            ["null", "bool", "i64", "f64", "string", "timestamp", "list"]
        );
    }

    #[test]
    fn timestamp_handles_signed_range() {
        let pre_epoch = Value::Timestamp(-86_400_000); // 1969-12-31T00:00:00Z
        let post_2262 = Value::Timestamp(i64::MAX);
        assert_eq!(pre_epoch.type_name(), "timestamp");
        assert_eq!(post_2262.type_name(), "timestamp");
    }

    #[test]
    fn list_is_recursive() {
        let v = Value::List(vec![
            Value::I64(1),
            Value::List(vec![Value::String("nested".into())]),
        ]);
        assert_eq!(v.type_name(), "list");
    }

    #[test]
    fn bincode_round_trip_preserves_discriminant_order() {
        // Fingerprints the variant order: the encoded discriminant
        // must equal the source-order index. If a future contributor
        // reorders the enum, this test will fail loudly — which is
        // the whole point. Adding new variants at the end is fine
        // and keeps this test passing.
        let cfg = bincode::config::standard();
        let cases = [
            (Value::Null, 0u8),
            (Value::Bool(true), 1),
            (Value::I64(0), 2),
            (Value::F64(0.0), 3),
            (Value::String(String::new()), 4),
            (Value::Timestamp(0), 5),
            (Value::List(vec![]), 6),
        ];
        for (v, expected_disc) in cases {
            let bytes = bincode::serde::encode_to_vec(&v, cfg).unwrap();
            assert_eq!(
                bytes[0],
                expected_disc,
                "discriminant for {} drifted",
                v.type_name()
            );
        }
    }
}
