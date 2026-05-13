//! Abstract syntax tree for the Cypher subset.
//!
//! v0.0.4-alpha.2 added directed relationship patterns (`(a)-[r]->(b)`)
//! and multi-pattern `MATCH p1, p2, ...`. `Statement::MatchReturn` and
//! `Statement::MatchDelete` now carry a `Vec<Pattern>`; each `Pattern`
//! is either a bare node or a path (node + chain of edge→node).

use std::collections::BTreeMap;

use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// `CREATE (n:Label {props})` — node only.
    Create { pattern: NodePattern },
    /// `MERGE (n:Label {id: $id, ...})` — must include an `id` property
    /// so the engine can look the node up. No-op on hit, insert on miss.
    Merge { pattern: NodePattern },
    /// `MATCH p1[, p2, ...] [WHERE expr] RETURN ... [ORDER BY expr [ASC|DESC]] [LIMIT n]`
    MatchReturn {
        patterns: Vec<Pattern>,
        where_clause: Option<Expr>,
        return_items: Vec<ReturnItem>,
        order_by: Option<OrderBy>,
        limit: Option<u64>,
    },
    /// `MATCH p1[, p2, ...] [WHERE expr] [DETACH] DELETE var`. When
    /// `detach` is true, the bound node is deleted along with every
    /// edge incident to it (Cypher cascade); when false, deleting a
    /// node that still has incident edges leaves them dangling
    /// (matches the engine's plain `delete_node`).
    MatchDelete {
        patterns: Vec<Pattern>,
        where_clause: Option<Expr>,
        variable: String,
        detach: bool,
    },
}

/// A `MATCH` clause is a comma-separated list of `Pattern`s. Each is
/// either a bare node `(n)` or a path `(a)-[r]->(b)[-[s]->(c)...]`.
/// Variables shared between patterns enforce equality constraints
/// during interpretation.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Node(NodePattern),
    Path {
        start: NodePattern,
        /// Successive `(edge, target_node)` pairs after `start`.
        /// `(a)-[r]->(b)-[s]->(c)` → start=a, chain=[(r,b),(s,c)].
        chain: Vec<(EdgePattern, NodePattern)>,
    },
}

/// `(variable[:Label][{props}])`. The variable is required; label and
/// properties are optional. `CREATE`/`MERGE` use the property block;
/// `MATCH` ignores it on `NodePattern` (predicates go in `WHERE`),
/// but the inline-property shorthand on `EdgePattern` IS honored by
/// `MATCH` — see `EdgePattern::properties`.
#[derive(Debug, Clone, PartialEq)]
pub struct NodePattern {
    pub variable: String,
    pub label: Option<String>,
    pub properties: BTreeMap<String, Expr>,
}

/// `-[variable[:Label][{props}]]->`. v0.0.4-alpha.2 ships the
/// directed-right form only; inverse / undirected patterns are
/// post-alpha.2 work and will reintroduce a direction discriminant
/// when they need it. Inline properties are honored by `MATCH` as
/// equality predicates (one fewer `WHERE` clause for the common case).
#[derive(Debug, Clone, PartialEq)]
pub struct EdgePattern {
    pub variable: String,
    pub label: Option<String>,
    pub properties: BTreeMap<String, Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReturnItem {
    /// `RETURN n`
    Variable(String),
    /// `RETURN n.name` or `RETURN n._morceau.foreshadow`. `path` is at
    /// least one element; the first element is the top-level key. A
    /// path of length > 1 with `_morceau` as the first element addresses
    /// the metadata-as-data namespace (see MORCEAU.md decision 19).
    Property { variable: String, path: Vec<String> },
    /// `RETURN count(*)` or `RETURN count(n)`. v0.0.4-alpha.4
    /// alongside `collect`. Collapses every binding row into one
    /// result row. The `target` is `None` for `count(*)` and
    /// `Some(variable)` for `count(n)`.
    ///
    /// No `GROUP BY` in alpha.4 — aggregate-only queries return one
    /// row, non-aggregate queries return one row per binding.
    /// Mixing aggregate + non-aggregate columns is not supported
    /// (no implicit grouping).
    Count { target: Option<String> },
    /// `RETURN collect(n)` or `RETURN collect(n.name)`. Gathers
    /// every projected value into a `Value::List` in one result row.
    Collect(CollectTarget),
}

/// What `collect()` projects from each binding row before gathering.
#[derive(Debug, Clone, PartialEq)]
pub enum CollectTarget {
    Variable(String),
    Property { variable: String, path: Vec<String> },
}

/// `ORDER BY <expr> [ASC|DESC]`. v0.0.4-alpha.4 ships one ordering
/// key only; multiple keys (`ORDER BY a.age DESC, a.name ASC`) wait
/// for a caller that needs them.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderBy {
    pub expr: Expr,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Param(String),
    /// `n.prop` or `n._morceau.foreshadow`. See `ReturnItem::Property`.
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
