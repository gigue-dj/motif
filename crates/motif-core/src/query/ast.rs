//! Abstract syntax tree for the Cypher subset.
//!
//! Intentionally minimal: a single bound variable per query,
//! single-pattern `MATCH`, `CREATE` / `MERGE` for nodes only, and a flat
//! expression tree for `WHERE`. v0.0.2-alpha.1 added multi-dot property
//! paths to support metadata-as-data namespaces (`n._motif.foreshadow`).

use std::collections::BTreeMap;

use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// `CREATE (n:Label {props})` — node only in v0.0.1.
    Create { pattern: NodePattern },
    /// `MERGE (n:Label {id: $id, ...})` — must include an `id` property
    /// so the engine can look the node up. No-op on hit, insert on miss.
    Merge { pattern: NodePattern },
    /// `MATCH (n[:Label]) [WHERE expr] RETURN ... [LIMIT n]`
    MatchReturn {
        pattern: NodePattern,
        where_clause: Option<Expr>,
        return_items: Vec<ReturnItem>,
        limit: Option<u64>,
    },
    /// `MATCH (n[:Label]) [WHERE expr] DELETE n`
    MatchDelete {
        pattern: NodePattern,
        where_clause: Option<Expr>,
        variable: String,
    },
}

/// `(variable[:Label][{props}])`. v0.0.1 requires the variable; the label
/// and properties are optional (used by `CREATE`/`MERGE`; `MATCH` ignores
/// the property block — predicates go in `WHERE`).
#[derive(Debug, Clone, PartialEq)]
pub struct NodePattern {
    pub variable: String,
    pub label: Option<String>,
    pub properties: BTreeMap<String, Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReturnItem {
    /// `RETURN n`
    Variable(String),
    /// `RETURN n.name` or `RETURN n._motif.foreshadow`. `path` is at
    /// least one element; the first element is the top-level key. A
    /// path of length > 1 with `_motif` as the first element addresses
    /// the metadata-as-data namespace (see MOTIF.md decision 19).
    Property { variable: String, path: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Param(String),
    /// `n.prop` or `n._motif.foreshadow`. See `ReturnItem::Property`.
    Property {
        variable: String,
        path: Vec<String>,
    },
    /// `id(n)` — the only built-in function in v0.0.1.
    IdOf(String),
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Not(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    And,
    Or,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
}
