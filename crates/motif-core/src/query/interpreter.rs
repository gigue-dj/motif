//! Direct interpreter: walks an AST and calls into [`Engine`]. No
//! planner, no optimizer — the AST is the plan.
//!
//! Match performance: constant-time when `WHERE id(n) = $x` is at
//! the top level of a conjunction (uses the engine's id index);
//! O(label_bucket) for `MATCH ()-[r:LABEL]->()` (uses
//! `edge_by_label`); O(N) scan otherwise. Inline-property predicates
//! on edge patterns filter within the label bucket — see LIMITATIONS
//! for the perf-debt note on the missing `from`-keyed adjacency
//! index.
//!
//! `_motif.<key>` paths in `WHERE` / `RETURN` resolve against
//! engine state rather than on-disk properties (MOTIF.md decision 19;
//! `_motif.foreshadow` → live foreshadow flag, `_motif.schema.version`
//! → current schema version, unknown keys → `Value::Null`).

use std::collections::BTreeMap;

use super::ast::{
    BinOp, CollectTarget, EdgePattern, Expr, NodePattern, OrderBy, Pattern, ReturnItem,
    SortDirection, Statement,
};
use super::result::{QueryResult, ResultCell};
use crate::engine::Engine;
use crate::graph::{Edge, Node};
use crate::value::Value;

pub type Params = BTreeMap<String, Value>;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum InterpretError {
    #[error("unknown parameter ${0}")]
    UnknownParam(String),
    #[error("unknown variable {0}")]
    UnknownVariable(String),
    #[error("expected boolean, got {0}")]
    ExpectedBool(&'static str),
    #[error("expected scalar value, got {0}")]
    ExpectedScalar(&'static str),
    #[error("type mismatch: {0}")]
    TypeMismatch(String),
    #[error("MERGE requires an `id` property")]
    MergeMissingId,
    #[error("MERGE/CREATE require a label")]
    MissingLabel,
    #[error("DELETE variable {0} does not match any pattern variable")]
    DeleteVariableMismatch(String),
    #[error(
        "DELETE on an edge variable is not supported (v0.0.4-alpha.2); delete via the engine API"
    )]
    DeleteEdgeUnsupported,
    #[error("nested property paths are not supported (got {0})")]
    NestedPath(String),
}

/// A single bound entity in a result row. Edge patterns bind both the
/// edge variable and its endpoint node variables.
#[derive(Debug, Clone)]
enum Binding {
    Node(Node),
    Edge(Edge),
}

impl Binding {
    fn id(&self) -> &str {
        match self {
            Binding::Node(n) => &n.id,
            Binding::Edge(e) => &e.id,
        }
    }
}

/// Per-row variable → binding map. Variable names are unique within
/// a row; cross-pattern shared variables must agree before a row is
/// emitted (enforced by `unify_bindings`).
type Bindings = BTreeMap<String, Binding>;

pub fn execute(
    engine: &mut Engine,
    stmt: &Statement,
    params: &Params,
) -> Result<QueryResult, InterpretError> {
    match stmt {
        Statement::Create { pattern } => exec_create(engine, pattern, params),
        Statement::Merge { pattern } => exec_merge(engine, pattern, params),
        Statement::MatchReturn {
            patterns,
            where_clause,
            return_items,
            order_by,
            limit,
        } => exec_match_return(
            engine,
            patterns,
            where_clause.as_ref(),
            return_items,
            order_by.as_ref(),
            *limit,
            params,
        ),
        Statement::MatchDelete {
            patterns,
            where_clause,
            variable,
            detach,
        } => exec_match_delete(
            engine,
            patterns,
            where_clause.as_ref(),
            variable,
            *detach,
            params,
        ),
    }
}

fn exec_create(
    engine: &mut Engine,
    pattern: &NodePattern,
    params: &Params,
) -> Result<QueryResult, InterpretError> {
    let label = pattern.label.clone().ok_or(InterpretError::MissingLabel)?;
    let props = eval_property_map(&pattern.properties, params)?;
    let id = require_id(&props)?;
    let mut node = Node::new(id, label);
    for (k, v) in props {
        if k != "id" {
            node.properties.insert(k, v);
        }
    }
    engine
        .insert_node(node)
        .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
    Ok(QueryResult::empty())
}

fn exec_merge(
    engine: &mut Engine,
    pattern: &NodePattern,
    params: &Params,
) -> Result<QueryResult, InterpretError> {
    let label = pattern.label.clone().ok_or(InterpretError::MissingLabel)?;
    let props = eval_property_map(&pattern.properties, params)?;
    let id = require_id(&props)?;

    if engine.has_node(&id) {
        return Ok(QueryResult::empty());
    }

    let mut node = Node::new(id, label);
    for (k, v) in props {
        if k != "id" {
            node.properties.insert(k, v);
        }
    }
    engine
        .insert_node(node)
        .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
    Ok(QueryResult::empty())
}

fn exec_match_return(
    engine: &mut Engine,
    patterns: &[Pattern],
    where_clause: Option<&Expr>,
    return_items: &[ReturnItem],
    order_by: Option<&OrderBy>,
    limit: Option<u64>,
    params: &Params,
) -> Result<QueryResult, InterpretError> {
    let aggregating = return_items.iter().any(is_aggregate);

    // LIMIT inside find_match_rows is only valid when neither ORDER
    // BY nor an aggregate needs the full row set first.
    let inline_limit = if aggregating || order_by.is_some() {
        None
    } else {
        limit
    };
    let mut rows = find_match_rows(engine, patterns, where_clause, params, inline_limit)?;

    if let Some(order) = order_by {
        sort_rows(&mut rows, order, params, engine)?;
    }

    let columns = return_items.iter().map(return_item_column_name).collect();

    let out_rows = if aggregating {
        project_aggregate_row(return_items, &rows, engine)?
    } else {
        let mut out: Vec<Vec<ResultCell>> = Vec::with_capacity(rows.len());
        for bindings in &rows {
            let mut row = Vec::with_capacity(return_items.len());
            for r in return_items {
                row.push(project(r, bindings, engine)?);
            }
            out.push(row);
            if let Some(l) = limit {
                if out.len() as u64 >= l {
                    break;
                }
            }
        }
        out
    };

    Ok(QueryResult {
        columns,
        rows: out_rows,
    })
}

fn is_aggregate(item: &ReturnItem) -> bool {
    matches!(item, ReturnItem::Count { .. } | ReturnItem::Collect(_))
}

fn return_item_column_name(r: &ReturnItem) -> String {
    match r {
        ReturnItem::Variable(v) => v.clone(),
        ReturnItem::Property { variable, path } => {
            let mut s = variable.clone();
            for seg in path {
                s.push('.');
                s.push_str(seg);
            }
            s
        }
        ReturnItem::Count { target } => match target {
            Some(v) => format!("count({v})"),
            None => "count(*)".to_string(),
        },
        ReturnItem::Collect(CollectTarget::Variable(v)) => format!("collect({v})"),
        ReturnItem::Collect(CollectTarget::Property { variable, path }) => {
            let mut s = format!("collect({variable}");
            for seg in path {
                s.push('.');
                s.push_str(seg);
            }
            s.push(')');
            s
        }
    }
}

fn project_aggregate_row(
    return_items: &[ReturnItem],
    rows: &[Bindings],
    engine: &Engine,
) -> Result<Vec<Vec<ResultCell>>, InterpretError> {
    let mut row = Vec::with_capacity(return_items.len());
    for item in return_items {
        let cell = match item {
            ReturnItem::Count { target: _ } => {
                // `count(n)` and `count(*)` both collapse to the row
                // count — alpha.4 doesn't track per-variable null
                // semantics (every binding row has every variable
                // bound, since unification is total).
                ResultCell::Value(Value::I64(rows.len() as i64))
            }
            ReturnItem::Collect(target) => {
                let mut list = Vec::with_capacity(rows.len());
                for bindings in rows {
                    let cell = collect_one(target, bindings, engine)?;
                    list.push(cell);
                }
                ResultCell::Value(Value::List(list))
            }
            _ => {
                return Err(InterpretError::TypeMismatch(
                    "mixing aggregate and non-aggregate columns is not supported \
                     (alpha.4 has no GROUP BY)"
                        .into(),
                ));
            }
        };
        row.push(cell);
    }
    Ok(vec![row])
}

fn collect_one(
    target: &CollectTarget,
    bindings: &Bindings,
    engine: &Engine,
) -> Result<Value, InterpretError> {
    match target {
        // `collect(n)` for a node / edge variable returns the id as a
        // string — a stopgap until `Value` grows Node / Edge variants
        // (tracked in LIMITATIONS as alpha.4 deferred). `collect(n.prop)`
        // is the well-shaped form callers should use today.
        CollectTarget::Variable(v) => match bindings.get(v) {
            Some(b @ (Binding::Node(_) | Binding::Edge(_))) => Ok(Value::String(b.id().to_owned())),
            None => Err(InterpretError::UnknownVariable(v.clone())),
        },
        CollectTarget::Property { variable, path } => match bindings.get(variable) {
            Some(Binding::Node(n)) => resolve_node_property_path(n, path, engine),
            Some(Binding::Edge(e)) => resolve_edge_property_path(e, path),
            None => Err(InterpretError::UnknownVariable(variable.clone())),
        },
    }
}

fn sort_rows(
    rows: &mut [Bindings],
    order: &OrderBy,
    params: &Params,
    engine: &Engine,
) -> Result<(), InterpretError> {
    // Pre-compute the sort key per row so the comparator doesn't
    // re-evaluate the expression at each comparison. Stable sort
    // preserves the natural pattern-walk order on equal keys.
    let mut keyed: Vec<(Value, Bindings)> = Vec::with_capacity(rows.len());
    for bindings in rows.iter() {
        let key = eval_expr(&order.expr, bindings, params, engine)?;
        keyed.push((key, bindings.clone()));
    }
    keyed.sort_by(|a, b| compare_for_sort(&a.0, &b.0, order.direction));
    for (i, (_, b)) in keyed.into_iter().enumerate() {
        rows[i] = b;
    }
    Ok(())
}

fn compare_for_sort(a: &Value, b: &Value, dir: SortDirection) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // Cypher-ish total ordering: Null sorts last (in ASC) so missing
    // values don't crowd the top of the result. Type-incomparable
    // pairs (e.g. String vs I64) compare equal — the caller's
    // pattern set is typically homogeneous in practice.
    let nat = match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Greater,
        (_, Value::Null) => Ordering::Less,
        (Value::I64(x), Value::I64(y)) => x.cmp(y),
        (Value::F64(x), Value::F64(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::I64(x), Value::F64(y)) => (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::F64(x), Value::I64(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Timestamp(x), Value::Timestamp(y)) => x.cmp(y),
        _ => Ordering::Equal,
    };
    match dir {
        SortDirection::Asc => nat,
        SortDirection::Desc => nat.reverse(),
    }
}

fn exec_match_delete(
    engine: &mut Engine,
    patterns: &[Pattern],
    where_clause: Option<&Expr>,
    variable: &str,
    detach: bool,
    params: &Params,
) -> Result<QueryResult, InterpretError> {
    let rows = find_match_rows(engine, patterns, where_clause, params, None)?;
    for bindings in rows {
        let Some(binding) = bindings.get(variable) else {
            return Err(InterpretError::DeleteVariableMismatch(variable.to_owned()));
        };
        match binding {
            Binding::Node(n) => {
                if detach {
                    engine
                        .delete_node_with_cascade(&n.id)
                        .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
                } else {
                    engine
                        .delete_node(&n.id)
                        .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
                }
            }
            Binding::Edge(_) => return Err(InterpretError::DeleteEdgeUnsupported),
        }
    }
    Ok(QueryResult::empty())
}

// ---- pattern matching ----

/// Produce the rows of bindings that satisfy every pattern in
/// `patterns` and the optional `where_clause`. Each row is a single
/// `Bindings` map covering every pattern variable.
fn find_match_rows(
    engine: &mut Engine,
    patterns: &[Pattern],
    where_clause: Option<&Expr>,
    params: &Params,
    limit: Option<u64>,
) -> Result<Vec<Bindings>, InterpretError> {
    let mut rows: Vec<Bindings> = vec![Bindings::new()];
    for pattern in patterns {
        let candidates = candidates_for_pattern(engine, pattern, where_clause, params)?;
        let mut next: Vec<Bindings> = Vec::new();
        for existing in &rows {
            for candidate in &candidates {
                if let Some(merged) = unify_bindings(existing, candidate) {
                    next.push(merged);
                }
            }
        }
        rows = next;
        if rows.is_empty() {
            return Ok(rows);
        }
    }

    let mut out: Vec<Bindings> = Vec::with_capacity(rows.len());
    for row in rows {
        if eval_predicate(where_clause, &row, params, engine)? {
            out.push(row);
            if let Some(l) = limit {
                if out.len() as u64 >= l {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Candidate bindings produced by a single pattern, before
/// cross-pattern unification.
fn candidates_for_pattern(
    engine: &mut Engine,
    pattern: &Pattern,
    where_clause: Option<&Expr>,
    params: &Params,
) -> Result<Vec<Bindings>, InterpretError> {
    match pattern {
        Pattern::Node(np) => node_candidates(engine, np, where_clause, params),
        Pattern::Path { start, chain } => {
            path_candidates(engine, start, chain, where_clause, params)
        }
    }
}

fn node_candidates(
    engine: &mut Engine,
    pattern: &NodePattern,
    where_clause: Option<&Expr>,
    params: &Params,
) -> Result<Vec<Bindings>, InterpretError> {
    // Fast path: WHERE id(n) = <scalar>. Constant-time index lookup.
    if let Some(id_value) = extract_id_predicate(where_clause, &pattern.variable, params)? {
        let by_id = engine
            .get_node(&id_value)
            .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
        let mut out = Vec::new();
        if let Some(node) = by_id {
            if pattern_matches_node(pattern, &node) {
                let mut row = Bindings::new();
                row.insert(pattern.variable.clone(), Binding::Node(node));
                out.push(row);
            }
        }
        return Ok(out);
    }

    // Slow path: scan.
    let all = engine
        .iter_nodes()
        .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
    let mut out = Vec::new();
    for node in all {
        if !pattern_matches_node(pattern, &node) {
            continue;
        }
        let mut row = Bindings::new();
        row.insert(pattern.variable.clone(), Binding::Node(node));
        out.push(row);
    }
    Ok(out)
}

/// Walk `(start)-[r1]->(n1)-[r2]->(n2)...` left-to-right, binding the
/// path's variables along the way. Pushes down `WHERE id(start) = $x`
/// to a single index lookup on the start node — pre-alpha.4 this
/// materialized every node in the namespace as a start candidate.
fn path_candidates(
    engine: &mut Engine,
    start: &NodePattern,
    chain: &[(EdgePattern, NodePattern)],
    where_clause: Option<&Expr>,
    params: &Params,
) -> Result<Vec<Bindings>, InterpretError> {
    let mut rows: Vec<Bindings> = Vec::new();
    if let Some(id_value) = extract_id_predicate(where_clause, &start.variable, params)? {
        if let Some(node) = engine
            .get_node(&id_value)
            .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?
        {
            if pattern_matches_node(start, &node) {
                let mut row = Bindings::new();
                row.insert(start.variable.clone(), Binding::Node(node));
                rows.push(row);
            }
        }
    } else {
        let start_nodes = engine
            .iter_nodes()
            .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
        for node in start_nodes {
            if !pattern_matches_node(start, &node) {
                continue;
            }
            let mut row = Bindings::new();
            row.insert(start.variable.clone(), Binding::Node(node));
            rows.push(row);
        }
    }

    // The "from" variable for hop k is the path's start for k=0 and
    // the previous hop's target thereafter. Tracked alongside the
    // chain walk so we don't re-scan the chain per row.
    let mut prev_var = start.variable.clone();
    for (edge_pat, target_pat) in chain {
        // Once per hop, not per edge — inline-property literals don't
        // depend on the bound row.
        let edge_props = eval_property_map(&edge_pat.properties, params)?;

        let mut next: Vec<Bindings> = Vec::new();
        for row in &rows {
            let prev_node = match row.get(&prev_var) {
                Some(Binding::Node(n)) => n,
                _ => continue,
            };
            // v0.0.4-alpha.4: O(degree) edge lookup via the
            // `edges_by_from` adjacency index, intersected with
            // `edge_by_label` when a label is declared. Pre-alpha.4
            // this walked every edge in the label bucket and
            // filtered by `edge.from` in memory.
            let candidate_edges = engine
                .iter_edges_from(&prev_node.id, edge_pat.label.as_deref())
                .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
            for edge in &candidate_edges {
                if !edge_pattern_matches(edge_pat, edge, &edge_props) {
                    continue;
                }
                let target = match engine
                    .get_node(&edge.to)
                    .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?
                {
                    Some(n) => n,
                    None => continue,
                };
                if !pattern_matches_node(target_pat, &target) {
                    continue;
                }
                let mut new_row = row.clone();
                new_row.insert(edge_pat.variable.clone(), Binding::Edge(edge.clone()));
                new_row.insert(target_pat.variable.clone(), Binding::Node(target));
                next.push(new_row);
            }
        }
        rows = next;
        prev_var = target_pat.variable.clone();
    }
    Ok(rows)
}

fn edge_pattern_matches(
    pattern: &EdgePattern,
    edge: &Edge,
    inline_props: &BTreeMap<String, Value>,
) -> bool {
    if let Some(label) = &pattern.label {
        if label != &edge.label {
            return false;
        }
    }
    for (k, expected) in inline_props {
        match edge.properties.get(k) {
            Some(actual) if values_equal(actual, expected) => {}
            _ => return false,
        }
    }
    true
}

/// Merge two partial binding rows. Shared variables must point at
/// the same entity (same id) — otherwise returns `None` (the join
/// fails). Variables present in only one side carry over. Failure
/// path skips the left-side clone so failed joins are O(shared_vars),
/// not O(|a|).
fn unify_bindings(a: &Bindings, b: &Bindings) -> Option<Bindings> {
    for (k, v) in b {
        if let Some(existing) = a.get(k) {
            if existing.id() != v.id() {
                return None;
            }
        }
    }
    let mut out = a.clone();
    for (k, v) in b {
        out.entry(k.clone()).or_insert_with(|| v.clone());
    }
    Some(out)
}

fn pattern_matches_node(pattern: &NodePattern, node: &Node) -> bool {
    !matches!(&pattern.label, Some(label) if label != &node.label)
}

fn extract_id_predicate(
    expr: Option<&Expr>,
    variable: &str,
    params: &Params,
) -> Result<Option<String>, InterpretError> {
    let Some(expr) = expr else { return Ok(None) };
    extract_id_in_expr(expr, variable, params)
}

/// Walk an expression looking for a top-level conjunction that
/// contains an `id(n) = <scalar>` clause. The walk is intentionally
/// conservative:
/// - Only top-level `AND` is descended (not `OR` — under disjunction
///   we'd have to enumerate two id sets, which we don't do).
/// - Only one id() match is honoured per query (the first match wins);
///   `id(n) = $x AND id(n) = $y` is therefore equivalent to using the
///   first scalar (the second predicate is re-checked in
///   `eval_predicate` and filters out a non-match).
fn extract_id_in_expr(
    expr: &Expr,
    variable: &str,
    params: &Params,
) -> Result<Option<String>, InterpretError> {
    match expr {
        Expr::Binary {
            op: BinOp::Eq,
            lhs,
            rhs,
        } => {
            let pair = match (&**lhs, &**rhs) {
                (Expr::IdOf(v), other) | (other, Expr::IdOf(v)) if v == variable => Some(other),
                _ => None,
            };
            if let Some(other) = pair {
                let v = eval_const(other, params)?;
                if let Value::String(s) = v {
                    return Ok(Some(s));
                }
            }
            Ok(None)
        }
        Expr::Binary {
            op: BinOp::And,
            lhs,
            rhs,
        } => {
            if let Some(found) = extract_id_in_expr(lhs, variable, params)? {
                return Ok(Some(found));
            }
            extract_id_in_expr(rhs, variable, params)
        }
        _ => Ok(None),
    }
}

/// Evaluate an expression that does not depend on a bound variable
/// (used by the id-predicate fast path and by inline edge-pattern
/// property evaluation).
fn eval_const(expr: &Expr, params: &Params) -> Result<Value, InterpretError> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Param(name) => params
            .get(name)
            .cloned()
            .ok_or_else(|| InterpretError::UnknownParam(name.clone())),
        _ => Err(InterpretError::ExpectedScalar(
            "non-constant in constant context",
        )),
    }
}

fn eval_predicate(
    expr: Option<&Expr>,
    bindings: &Bindings,
    params: &Params,
    engine: &Engine,
) -> Result<bool, InterpretError> {
    let Some(expr) = expr else { return Ok(true) };
    let v = eval_expr(expr, bindings, params, engine)?;
    match v {
        Value::Bool(b) => Ok(b),
        Value::Null => Ok(false),
        other => Err(InterpretError::ExpectedBool(static_type_of(&other))),
    }
}

fn eval_expr(
    expr: &Expr,
    bindings: &Bindings,
    params: &Params,
    engine: &Engine,
) -> Result<Value, InterpretError> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Param(name) => params
            .get(name)
            .cloned()
            .ok_or_else(|| InterpretError::UnknownParam(name.clone())),
        Expr::IdOf(v) => match bindings.get(v) {
            Some(b) => Ok(Value::String(b.id().to_owned())),
            None => Err(InterpretError::UnknownVariable(v.clone())),
        },
        Expr::Property { variable, path } => match bindings.get(variable) {
            Some(Binding::Node(n)) => resolve_node_property_path(n, path, engine),
            Some(Binding::Edge(e)) => resolve_edge_property_path(e, path),
            None => Err(InterpretError::UnknownVariable(variable.clone())),
        },
        Expr::Not(inner) => {
            let v = eval_expr(inner, bindings, params, engine)?;
            match v {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                Value::Null => Ok(Value::Null),
                other => Err(InterpretError::ExpectedBool(static_type_of(&other))),
            }
        }
        Expr::Binary { op, lhs, rhs } => {
            let l = eval_expr(lhs, bindings, params, engine)?;
            let r = eval_expr(rhs, bindings, params, engine)?;
            apply_binop(*op, l, r)
        }
    }
}

/// Resolve a property path against a bound node. Single-segment paths
/// hit the node's user properties; `_motif.<rest>` paths hit the
/// metadata-as-data namespace (engine state). Arbitrary-depth paths
/// are allowed only inside the `_motif` namespace; user-property
/// nesting (`n.address.city`) is a v0.0.5+ shape.
fn resolve_node_property_path(
    node: &Node,
    path: &[String],
    engine: &Engine,
) -> Result<Value, InterpretError> {
    match path {
        [key] => Ok(node.properties.get(key).cloned().unwrap_or(Value::Null)),
        [namespace, rest @ ..] if namespace == "_motif" => Ok(motif_metadata(node, rest, engine)),
        _ => Err(InterpretError::NestedPath(path.join("."))),
    }
}

/// Resolve a property path against a bound edge. Edges don't (yet)
/// participate in the `_motif` namespace — the foreshadow flag is
/// engine-wide and reachable via the node-bound path; future alphas
/// may expose `r._motif.X` if a use case appears.
fn resolve_edge_property_path(edge: &Edge, path: &[String]) -> Result<Value, InterpretError> {
    match path {
        [key] => Ok(edge.properties.get(key).cloned().unwrap_or(Value::Null)),
        _ => Err(InterpretError::NestedPath(path.join("."))),
    }
}

/// `_motif.<...>` lookups. Unknown keys return `Value::Null` rather
/// than an error so hosts can probe the namespace forward-compatibly.
fn motif_metadata(node: &Node, path: &[String], engine: &Engine) -> Value {
    match path {
        [k] if k == "foreshadow" => Value::Bool(engine.is_foreshadow(&node.id)),
        [a, b] if a == "schema" && b == "version" => engine
            .current_schema()
            .map(|s| Value::I64(s.version as i64))
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn apply_binop(op: BinOp, l: Value, r: Value) -> Result<Value, InterpretError> {
    use BinOp::*;
    match op {
        And => match (l, r) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a && b)),
            (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
            (a, b) => Err(InterpretError::TypeMismatch(format!(
                "AND requires booleans, got {} and {}",
                static_type_of(&a),
                static_type_of(&b)
            ))),
        },
        Or => match (l, r) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a || b)),
            (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
            (a, b) => Err(InterpretError::TypeMismatch(format!(
                "OR requires booleans, got {} and {}",
                static_type_of(&a),
                static_type_of(&b)
            ))),
        },
        Eq => Ok(Value::Bool(values_equal(&l, &r))),
        NotEq => Ok(Value::Bool(!values_equal(&l, &r))),
        Lt | Gt | LtEq | GtEq => compare(op, &l, &r),
    }
}

fn values_equal(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::I64(a), Value::I64(b)) => a == b,
        (Value::F64(a), Value::F64(b)) => a == b,
        (Value::I64(a), Value::F64(b)) | (Value::F64(b), Value::I64(a)) => (*a as f64) == *b,
        (Value::String(a), Value::String(b)) => a == b,
        _ => false,
    }
}

fn compare(op: BinOp, l: &Value, r: &Value) -> Result<Value, InterpretError> {
    let ord = match (l, r) {
        (Value::I64(a), Value::I64(b)) => (*a as f64).partial_cmp(&(*b as f64)),
        (Value::F64(a), Value::F64(b)) => a.partial_cmp(b),
        (Value::I64(a), Value::F64(b)) => (*a as f64).partial_cmp(b),
        (Value::F64(a), Value::I64(b)) => a.partial_cmp(&(*b as f64)),
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        (Value::Null, _) | (_, Value::Null) => return Ok(Value::Null),
        (a, b) => {
            return Err(InterpretError::TypeMismatch(format!(
                "cannot compare {} and {}",
                static_type_of(a),
                static_type_of(b)
            )));
        }
    };
    let Some(ord) = ord else {
        return Ok(Value::Null);
    };
    use std::cmp::Ordering;
    let result = matches!(
        (op, ord),
        (BinOp::Lt, Ordering::Less)
            | (BinOp::LtEq, Ordering::Less | Ordering::Equal)
            | (BinOp::Gt, Ordering::Greater)
            | (BinOp::GtEq, Ordering::Greater | Ordering::Equal)
    );
    Ok(Value::Bool(result))
}

fn project(
    item: &ReturnItem,
    bindings: &Bindings,
    engine: &Engine,
) -> Result<ResultCell, InterpretError> {
    match item {
        ReturnItem::Variable(v) => match bindings.get(v) {
            Some(Binding::Node(n)) => Ok(ResultCell::Node(n.clone())),
            Some(Binding::Edge(e)) => Ok(ResultCell::Edge(e.clone())),
            None => Err(InterpretError::UnknownVariable(v.clone())),
        },
        ReturnItem::Property { variable, path } => match bindings.get(variable) {
            Some(Binding::Node(n)) => Ok(ResultCell::Value(resolve_node_property_path(
                n, path, engine,
            )?)),
            Some(Binding::Edge(e)) => Ok(ResultCell::Value(resolve_edge_property_path(e, path)?)),
            None => Err(InterpretError::UnknownVariable(variable.clone())),
        },
        ReturnItem::Count { .. } | ReturnItem::Collect(_) => {
            unreachable!("aggregates dispatch to project_aggregate_row in exec_match_return")
        }
    }
}

fn eval_property_map(
    props: &BTreeMap<String, Expr>,
    params: &Params,
) -> Result<BTreeMap<String, Value>, InterpretError> {
    let mut out = BTreeMap::new();
    for (k, v) in props {
        out.insert(k.clone(), eval_const(v, params)?);
    }
    Ok(out)
}

fn require_id(props: &BTreeMap<String, Value>) -> Result<String, InterpretError> {
    match props.get("id") {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(InterpretError::TypeMismatch(
            "`id` property must be a string".into(),
        )),
        None => Err(InterpretError::MergeMissingId),
    }
}

fn static_type_of(v: &Value) -> &'static str {
    v.type_name()
}
