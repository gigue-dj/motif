//! Direct interpreter: walks an AST and calls into [`Engine`]. No
//! planner, no optimizer — v0.0.1 is small enough that the AST is the
//! plan.
//!
//! Pattern matching for `MATCH` is constant-time when the `WHERE`
//! contains an `id(n) = $x` (or `id(n) = literal`) predicate, and an
//! O(N) scan otherwise. Label and property predicates filter the scan.

use std::collections::BTreeMap;

use super::ast::{BinOp, Expr, NodePattern, ReturnItem, Statement};
use super::result::{QueryResult, ResultCell};
use crate::engine::Engine;
use crate::graph::Node;
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
    #[error("DELETE variable {0} does not match MATCH variable")]
    DeleteVariableMismatch(String),
}

pub fn execute(
    engine: &mut Engine,
    stmt: &Statement,
    params: &Params,
) -> Result<QueryResult, InterpretError> {
    match stmt {
        Statement::Create { pattern } => exec_create(engine, pattern, params),
        Statement::Merge { pattern } => exec_merge(engine, pattern, params),
        Statement::MatchReturn {
            pattern,
            where_clause,
            return_items,
            limit,
        } => exec_match_return(
            engine,
            pattern,
            where_clause.as_ref(),
            return_items,
            *limit,
            params,
        ),
        Statement::MatchDelete {
            pattern,
            where_clause,
            variable,
        } => exec_match_delete(engine, pattern, where_clause.as_ref(), variable, params),
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
    // Engine errors (duplicate id, etc.) bubble up via Engine::query.
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
    pattern: &NodePattern,
    where_clause: Option<&Expr>,
    return_items: &[ReturnItem],
    limit: Option<u64>,
    params: &Params,
) -> Result<QueryResult, InterpretError> {
    let matches = find_matches(engine, pattern, where_clause, params, limit)?;

    let columns = return_items
        .iter()
        .map(|r| match r {
            ReturnItem::Variable(v) => v.clone(),
            ReturnItem::Property { variable, key } => format!("{variable}.{key}"),
        })
        .collect();

    let mut rows = Vec::with_capacity(matches.len());
    for node in matches {
        let mut row = Vec::with_capacity(return_items.len());
        for r in return_items {
            row.push(project(r, &pattern.variable, &node)?);
        }
        rows.push(row);
    }
    Ok(QueryResult { columns, rows })
}

fn exec_match_delete(
    engine: &mut Engine,
    pattern: &NodePattern,
    where_clause: Option<&Expr>,
    variable: &str,
    params: &Params,
) -> Result<QueryResult, InterpretError> {
    if variable != pattern.variable {
        return Err(InterpretError::DeleteVariableMismatch(variable.to_owned()));
    }
    let matches = find_matches(engine, pattern, where_clause, params, None)?;
    for node in matches {
        engine
            .delete_node(&node.id)
            .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
    }
    Ok(QueryResult::empty())
}

// ---- pattern matching ----

fn find_matches(
    engine: &mut Engine,
    pattern: &NodePattern,
    where_clause: Option<&Expr>,
    params: &Params,
    limit: Option<u64>,
) -> Result<Vec<Node>, InterpretError> {
    // Fast path: WHERE id(n) = <scalar>. Constant-time index lookup.
    if let Some(id_value) = extract_id_predicate(where_clause, &pattern.variable, params)? {
        let by_id = engine
            .get_node(&id_value)
            .map_err(|e| InterpretError::TypeMismatch(e.to_string()))?;
        let mut out = Vec::new();
        if let Some(node) = by_id {
            if pattern_matches_node(pattern, &node)
                && eval_predicate(where_clause, &pattern.variable, &node, params)?
            {
                out.push(node);
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
        if !eval_predicate(where_clause, &pattern.variable, &node, params)? {
            continue;
        }
        out.push(node);
        if let Some(l) = limit {
            if out.len() as u64 >= l {
                break;
            }
        }
    }
    Ok(out)
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
    if let Expr::Binary {
        op: BinOp::Eq,
        lhs,
        rhs,
    } = expr
    {
        let pair = match (&**lhs, &**rhs) {
            (Expr::IdOf(v), other) | (other, Expr::IdOf(v)) if v == variable => Some(other),
            _ => None,
        };
        if let Some(other) = pair {
            // Resolve `other` to a string id without needing a node binding.
            let v = eval_const(other, params)?;
            if let Value::String(s) = v {
                return Ok(Some(s));
            }
        }
    }
    Ok(None)
}

/// Evaluate an expression that does not depend on a bound variable
/// (used by the id-predicate fast path).
fn eval_const(expr: &Expr, params: &Params) -> Result<Value, InterpretError> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Param(name) => params
            .get(name)
            .cloned()
            .ok_or_else(|| InterpretError::UnknownParam(name.clone())),
        _ => Err(InterpretError::ExpectedScalar(
            "non-constant in id() comparison",
        )),
    }
}

fn eval_predicate(
    expr: Option<&Expr>,
    variable: &str,
    node: &Node,
    params: &Params,
) -> Result<bool, InterpretError> {
    let Some(expr) = expr else { return Ok(true) };
    let v = eval_expr(expr, variable, node, params)?;
    match v {
        Value::Bool(b) => Ok(b),
        Value::Null => Ok(false),
        other => Err(InterpretError::ExpectedBool(static_type_of(&other))),
    }
}

fn eval_expr(
    expr: &Expr,
    variable: &str,
    node: &Node,
    params: &Params,
) -> Result<Value, InterpretError> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Param(name) => params
            .get(name)
            .cloned()
            .ok_or_else(|| InterpretError::UnknownParam(name.clone())),
        Expr::IdOf(v) if v == variable => Ok(Value::String(node.id.clone())),
        Expr::IdOf(other) => Err(InterpretError::UnknownVariable(other.clone())),
        Expr::Property { variable: v, key } if v == variable => {
            Ok(node.properties.get(key).cloned().unwrap_or(Value::Null))
        }
        Expr::Property { variable: v, .. } => Err(InterpretError::UnknownVariable(v.clone())),
        Expr::Not(inner) => {
            let v = eval_expr(inner, variable, node, params)?;
            match v {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                Value::Null => Ok(Value::Null),
                other => Err(InterpretError::ExpectedBool(static_type_of(&other))),
            }
        }
        Expr::Binary { op, lhs, rhs } => {
            let l = eval_expr(lhs, variable, node, params)?;
            let r = eval_expr(rhs, variable, node, params)?;
            apply_binop(*op, l, r)
        }
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

fn project(item: &ReturnItem, var: &str, node: &Node) -> Result<ResultCell, InterpretError> {
    match item {
        ReturnItem::Variable(v) if v == var => Ok(ResultCell::Node(node.clone())),
        ReturnItem::Variable(v) => Err(InterpretError::UnknownVariable(v.clone())),
        ReturnItem::Property { variable: v, key } if v == var => Ok(ResultCell::Value(
            node.properties.get(key).cloned().unwrap_or(Value::Null),
        )),
        ReturnItem::Property { variable: v, .. } => Err(InterpretError::UnknownVariable(v.clone())),
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
