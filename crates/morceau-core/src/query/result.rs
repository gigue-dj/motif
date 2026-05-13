//! Query result shape.

use serde::{Deserialize, Serialize};

use crate::graph::{Edge, Node};
use crate::value::Value;

/// A single cell in a result row. `RETURN n` produces a `Node`,
/// `RETURN r` (where `r` is bound by an edge pattern) produces an
/// `Edge`, and `RETURN n.prop` / `RETURN r.prop` produce a `Value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResultCell {
    Value(Value),
    Node(Node),
    Edge(Edge),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<ResultCell>>,
}

impl QueryResult {
    pub fn empty() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }
}
