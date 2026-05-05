//! Abstract syntax tree for the v0.0.1 Cypher subset.
//!
//! Intentionally minimal: a single bound variable per query, single-pattern
//! `MATCH`, `CREATE` / `MERGE` for nodes only (edge create lands in
//! alpha.5), and a flat expression tree for `WHERE`.

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
    /// `RETURN n.name`
    Property { variable: String, key: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Param(String),
    /// `n.prop`
    Property {
        variable: String,
        key: String,
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
