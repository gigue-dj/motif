//! Recursive-descent parser for the v0.0.1 Cypher subset. Consumes a
//! [`crate::query::lexer::Spanned`] stream and produces a [`Statement`].
//!
//! The grammar is summarised in `query/mod.rs`. Errors carry the byte
//! offset of the offending token (or end-of-input) and a short message.

use std::collections::BTreeMap;

use super::ast::{BinOp, EdgePattern, Expr, NodePattern, Pattern, ReturnItem, Statement};
use super::lexer::{LexError, Spanned, Token};
use crate::value::Value;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseError {
    #[error("lex error: {0}")]
    Lex(#[from] LexError),
    #[error("unexpected token {token} at offset {offset} (expected {expected})")]
    Unexpected {
        token: String,
        expected: String,
        offset: usize,
    },
    #[error("unexpected end of input (expected {expected})")]
    UnexpectedEnd { expected: String },
    #[error("unsupported statement: {0}")]
    Unsupported(String),
}

struct Parser<'a> {
    tokens: &'a [Spanned],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|s| &s.token)
    }

    fn peek_offset(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map(|s| s.offset)
            .unwrap_or_else(|| self.tokens.last().map(|s| s.offset).unwrap_or(0))
    }

    fn advance(&mut self) -> Option<&'a Spanned> {
        let s = self.tokens.get(self.pos);
        if s.is_some() {
            self.pos += 1;
        }
        s
    }

    fn expect(&mut self, expected: &Token, label: &str) -> Result<(), ParseError> {
        match self.peek() {
            Some(t) if std::mem::discriminant(t) == std::mem::discriminant(expected) => {
                self.advance();
                Ok(())
            }
            Some(t) => Err(ParseError::Unexpected {
                token: t.to_string(),
                expected: label.to_owned(),
                offset: self.peek_offset(),
            }),
            None => Err(ParseError::UnexpectedEnd {
                expected: label.to_owned(),
            }),
        }
    }

    fn expect_ident(&mut self, label: &str) -> Result<String, ParseError> {
        match self.peek() {
            Some(Token::Ident(s)) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            Some(t) => Err(ParseError::Unexpected {
                token: t.to_string(),
                expected: label.to_owned(),
                offset: self.peek_offset(),
            }),
            None => Err(ParseError::UnexpectedEnd {
                expected: label.to_owned(),
            }),
        }
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match self.peek() {
            Some(Token::Create) => {
                self.advance();
                let pattern = self.parse_node_pattern()?;
                self.expect_end()?;
                Ok(Statement::Create { pattern })
            }
            Some(Token::Merge) => {
                self.advance();
                let pattern = self.parse_node_pattern()?;
                self.expect_end()?;
                Ok(Statement::Merge { pattern })
            }
            Some(Token::Match) => {
                self.advance();
                let patterns = self.parse_pattern_list()?;
                let where_clause = if matches!(self.peek(), Some(Token::Where)) {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                match self.peek() {
                    Some(Token::Return) => {
                        self.advance();
                        let return_items = self.parse_return_items()?;
                        let limit = if matches!(self.peek(), Some(Token::Limit)) {
                            self.advance();
                            Some(self.parse_unsigned_int()?)
                        } else {
                            None
                        };
                        self.expect_end()?;
                        Ok(Statement::MatchReturn {
                            patterns,
                            where_clause,
                            return_items,
                            limit,
                        })
                    }
                    Some(Token::Delete) | Some(Token::Detach) => {
                        let detach = if matches!(self.peek(), Some(Token::Detach)) {
                            self.advance();
                            true
                        } else {
                            false
                        };
                        self.expect(&Token::Delete, "DELETE")?;
                        let variable = self.expect_ident("variable to DELETE")?;
                        self.expect_end()?;
                        Ok(Statement::MatchDelete {
                            patterns,
                            where_clause,
                            variable,
                            detach,
                        })
                    }
                    Some(t) => Err(ParseError::Unexpected {
                        token: t.to_string(),
                        expected: "RETURN, DELETE, or DETACH DELETE".into(),
                        offset: self.peek_offset(),
                    }),
                    None => Err(ParseError::UnexpectedEnd {
                        expected: "RETURN, DELETE, or DETACH DELETE".into(),
                    }),
                }
            }
            Some(t) => Err(ParseError::Unsupported(t.to_string())),
            None => Err(ParseError::UnexpectedEnd {
                expected: "statement".into(),
            }),
        }
    }

    fn parse_node_pattern(&mut self) -> Result<NodePattern, ParseError> {
        self.expect(&Token::LParen, "(")?;
        let variable = self.expect_ident("pattern variable")?;
        let label = if matches!(self.peek(), Some(Token::Colon)) {
            self.advance();
            Some(self.expect_ident("label")?)
        } else {
            None
        };
        let properties = if matches!(self.peek(), Some(Token::LBrace)) {
            self.parse_property_map()?
        } else {
            BTreeMap::new()
        };
        self.expect(&Token::RParen, ")")?;
        Ok(NodePattern {
            variable,
            label,
            properties,
        })
    }

    /// `MATCH p1[, p2, ...]` — comma-separated patterns.
    fn parse_pattern_list(&mut self) -> Result<Vec<Pattern>, ParseError> {
        let mut patterns = vec![self.parse_pattern()?];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.advance();
            patterns.push(self.parse_pattern()?);
        }
        Ok(patterns)
    }

    /// A single pattern: `(node)` or `(start)-[e]->(b)-[f]->(c)...`.
    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        let start = self.parse_node_pattern()?;
        let mut chain = Vec::new();
        while matches!(self.peek(), Some(Token::Dash)) {
            let edge = self.parse_edge_pattern()?;
            let target = self.parse_node_pattern()?;
            chain.push((edge, target));
        }
        if chain.is_empty() {
            Ok(Pattern::Node(start))
        } else {
            Ok(Pattern::Path { start, chain })
        }
    }

    /// `-[variable[:Label][{props}]]->`. The caller's `peek` must have
    /// already seen the leading `-` (so the dispatch in `parse_pattern`
    /// reached this function); this function consumes it. v0.0.4-alpha.2
    /// supports only the directed-right form `->`; inverse `<-` is
    /// post-alpha.2.
    fn parse_edge_pattern(&mut self) -> Result<EdgePattern, ParseError> {
        self.expect(&Token::Dash, "-")?;
        self.expect(&Token::LBracket, "[")?;
        let variable = self.expect_ident("relationship variable")?;
        let label = if matches!(self.peek(), Some(Token::Colon)) {
            self.advance();
            Some(self.expect_ident("relationship label")?)
        } else {
            None
        };
        let properties = if matches!(self.peek(), Some(Token::LBrace)) {
            self.parse_property_map()?
        } else {
            BTreeMap::new()
        };
        self.expect(&Token::RBracket, "]")?;
        // The closing arrow `]->` lexes as RBracket + Arrow because
        // the lexer collapsed `-` + `>` into a single Arrow token at
        // tokenisation time.
        self.expect(&Token::Arrow, "->")?;
        Ok(EdgePattern {
            variable,
            label,
            properties,
        })
    }

    fn parse_property_map(&mut self) -> Result<BTreeMap<String, Expr>, ParseError> {
        self.expect(&Token::LBrace, "{")?;
        let mut props = BTreeMap::new();
        if matches!(self.peek(), Some(Token::RBrace)) {
            self.advance();
            return Ok(props);
        }
        loop {
            let key = self.expect_ident("property key")?;
            self.expect(&Token::Colon, ":")?;
            let val = self.parse_expr()?;
            props.insert(key, val);
            match self.peek() {
                Some(Token::Comma) => {
                    self.advance();
                }
                Some(Token::RBrace) => {
                    self.advance();
                    return Ok(props);
                }
                Some(t) => {
                    return Err(ParseError::Unexpected {
                        token: t.to_string(),
                        expected: ", or }".into(),
                        offset: self.peek_offset(),
                    });
                }
                None => {
                    return Err(ParseError::UnexpectedEnd {
                        expected: ", or }".into(),
                    });
                }
            }
        }
    }

    fn parse_return_items(&mut self) -> Result<Vec<ReturnItem>, ParseError> {
        let mut items = Vec::new();
        loop {
            let var = self.expect_ident("return variable")?;
            let item = if matches!(self.peek(), Some(Token::Dot)) {
                let path = self.parse_property_path()?;
                ReturnItem::Property {
                    variable: var,
                    path,
                }
            } else {
                ReturnItem::Variable(var)
            };
            items.push(item);
            if matches!(self.peek(), Some(Token::Comma)) {
                self.advance();
            } else {
                return Ok(items);
            }
        }
    }

    /// Consume `.ident (.ident)*` and return the path. The leading dot
    /// must already have been peeked (but not consumed) by the caller.
    fn parse_property_path(&mut self) -> Result<Vec<String>, ParseError> {
        debug_assert!(matches!(self.peek(), Some(Token::Dot)));
        let mut path = Vec::new();
        while matches!(self.peek(), Some(Token::Dot)) {
            self.advance();
            path.push(self.expect_ident("property key")?);
        }
        Ok(path)
    }

    fn parse_unsigned_int(&mut self) -> Result<u64, ParseError> {
        match self.peek() {
            Some(Token::Integer(i)) if *i >= 0 => {
                let v = *i as u64;
                self.advance();
                Ok(v)
            }
            Some(t) => Err(ParseError::Unexpected {
                token: t.to_string(),
                expected: "non-negative integer".into(),
                offset: self.peek_offset(),
            }),
            None => Err(ParseError::UnexpectedEnd {
                expected: "non-negative integer".into(),
            }),
        }
    }

    fn expect_end(&mut self) -> Result<(), ParseError> {
        if let Some(t) = self.peek() {
            return Err(ParseError::Unexpected {
                token: t.to_string(),
                expected: "end of statement".into(),
                offset: self.peek_offset(),
            });
        }
        Ok(())
    }

    // ---- expressions: OR > AND > NOT > comparison > primary ----

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.advance();
            let rhs = self.parse_and()?;
            lhs = Expr::Binary {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_not()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.advance();
            let rhs = self.parse_not()?;
            lhs = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_not(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.advance();
            let inner = self.parse_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.parse_primary()?;
        let op = match self.peek() {
            Some(Token::Eq) => BinOp::Eq,
            Some(Token::NotEq) => BinOp::NotEq,
            Some(Token::Lt) => BinOp::Lt,
            Some(Token::Gt) => BinOp::Gt,
            Some(Token::LtEq) => BinOp::LtEq,
            Some(Token::GtEq) => BinOp::GtEq,
            _ => return Ok(lhs),
        };
        self.advance();
        let rhs = self.parse_primary()?;
        Ok(Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        })
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Token::LParen) => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen, ")")?;
                Ok(inner)
            }
            Some(Token::Integer(i)) => {
                self.advance();
                Ok(Expr::Literal(Value::I64(i)))
            }
            Some(Token::Float(f)) => {
                self.advance();
                Ok(Expr::Literal(Value::F64(f)))
            }
            Some(Token::String(s)) => {
                self.advance();
                Ok(Expr::Literal(Value::String(s)))
            }
            Some(Token::True) => {
                self.advance();
                Ok(Expr::Literal(Value::Bool(true)))
            }
            Some(Token::False) => {
                self.advance();
                Ok(Expr::Literal(Value::Bool(false)))
            }
            Some(Token::Null) => {
                self.advance();
                Ok(Expr::Literal(Value::Null))
            }
            Some(Token::Param(name)) => {
                self.advance();
                Ok(Expr::Param(name))
            }
            Some(Token::Ident(name)) => {
                self.advance();
                // `id(x)` is the only call form in v0.0.1.
                if name == "id" && matches!(self.peek(), Some(Token::LParen)) {
                    self.advance();
                    let arg = self.expect_ident("variable inside id(...)")?;
                    self.expect(&Token::RParen, ")")?;
                    return Ok(Expr::IdOf(arg));
                }
                if matches!(self.peek(), Some(Token::Dot)) {
                    let path = self.parse_property_path()?;
                    Ok(Expr::Property {
                        variable: name,
                        path,
                    })
                } else {
                    Err(ParseError::Unexpected {
                        token: name,
                        expected: "function call or property access".into(),
                        offset: self.peek_offset(),
                    })
                }
            }
            Some(t) => Err(ParseError::Unexpected {
                token: t.to_string(),
                expected: "expression".into(),
                offset: self.peek_offset(),
            }),
            None => Err(ParseError::UnexpectedEnd {
                expected: "expression".into(),
            }),
        }
    }
}

pub fn parse_tokens(tokens: &[Spanned]) -> Result<Statement, ParseError> {
    let mut p = Parser { tokens, pos: 0 };
    p.parse_statement()
}

#[cfg(test)]
mod tests {
    use super::super::lexer::lex;
    use super::*;

    fn parse(s: &str) -> Result<Statement, ParseError> {
        let toks = lex(s)?;
        parse_tokens(&toks)
    }

    #[test]
    fn parses_create_node_with_props() {
        let s = parse("CREATE (n:Person {name: 'Alice', age: 30})").unwrap();
        match s {
            Statement::Create { pattern } => {
                assert_eq!(pattern.variable, "n");
                assert_eq!(pattern.label.as_deref(), Some("Person"));
                assert_eq!(pattern.properties.len(), 2);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parses_match_with_id_predicate() {
        let s = parse("MATCH (n) WHERE id(n) = $x RETURN n").unwrap();
        match s {
            Statement::MatchReturn {
                patterns,
                where_clause,
                return_items,
                limit,
            } => {
                assert_eq!(patterns.len(), 1);
                assert!(matches!(
                    &patterns[0],
                    Pattern::Node(NodePattern { variable, .. }) if variable == "n"
                ));
                assert!(where_clause.is_some());
                assert_eq!(return_items, vec![ReturnItem::Variable("n".into())]);
                assert!(limit.is_none());
            }
            _ => panic!("expected MatchReturn"),
        }
    }

    #[test]
    fn parses_match_delete() {
        let s = parse("MATCH (n) WHERE id(n) = $x DELETE n").unwrap();
        match s {
            Statement::MatchDelete { variable, .. } => assert_eq!(variable, "n"),
            _ => panic!("expected MatchDelete"),
        }
    }

    #[test]
    fn parses_single_edge_pattern() {
        let s = parse("MATCH (a)-[r]->(b) RETURN r").unwrap();
        let Statement::MatchReturn { patterns, .. } = s else {
            panic!("expected MatchReturn");
        };
        assert_eq!(patterns.len(), 1);
        let Pattern::Path { start, chain } = &patterns[0] else {
            panic!("expected Path");
        };
        assert_eq!(start.variable, "a");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0.variable, "r");
        assert!(chain[0].0.label.is_none());
        assert_eq!(chain[0].1.variable, "b");
    }

    #[test]
    fn parses_edge_pattern_with_label_and_inline_props() {
        let s = parse("MATCH (a)-[r:KNOWS {since: 2020}]->(b) RETURN r").unwrap();
        let Statement::MatchReturn { patterns, .. } = s else {
            panic!()
        };
        let Pattern::Path { chain, .. } = &patterns[0] else {
            panic!()
        };
        assert_eq!(chain[0].0.label.as_deref(), Some("KNOWS"));
        assert_eq!(chain[0].0.properties.len(), 1);
        assert!(chain[0].0.properties.contains_key("since"));
    }

    #[test]
    fn parses_multi_hop_path() {
        let s = parse("MATCH (a)-[r:KNOWS]->(b)-[s:FOLLOWS]->(c) RETURN c").unwrap();
        let Statement::MatchReturn { patterns, .. } = s else {
            panic!()
        };
        let Pattern::Path { chain, .. } = &patterns[0] else {
            panic!()
        };
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].0.label.as_deref(), Some("KNOWS"));
        assert_eq!(chain[1].0.label.as_deref(), Some("FOLLOWS"));
    }

    #[test]
    fn parses_multi_pattern_match() {
        let s = parse("MATCH (a)-[r]->(b), (b)-[s]->(c) RETURN c").unwrap();
        let Statement::MatchReturn { patterns, .. } = s else {
            panic!()
        };
        assert_eq!(patterns.len(), 2);
    }

    #[test]
    fn parses_merge() {
        let s = parse("MERGE (n:Person {id: $id, name: 'Bob'})").unwrap();
        assert!(matches!(s, Statement::Merge { .. }));
    }

    #[test]
    fn parses_return_with_limit_and_property() {
        let s = parse("MATCH (n) RETURN n.name, n LIMIT 10").unwrap();
        if let Statement::MatchReturn {
            return_items,
            limit,
            ..
        } = s
        {
            assert_eq!(
                return_items,
                vec![
                    ReturnItem::Property {
                        variable: "n".into(),
                        path: vec!["name".into()],
                    },
                    ReturnItem::Variable("n".into()),
                ]
            );
            assert_eq!(limit, Some(10));
        } else {
            panic!()
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("CREATE 42").is_err());
        assert!(parse("MATCH (n) RETURN").is_err());
        assert!(parse("MATCH (n) RETURN n EXTRA").is_err());
    }

    #[test]
    fn parses_motif_metadata_path() {
        let s = parse("MATCH (n) WHERE n._motif.foreshadow = true RETURN n").unwrap();
        let Statement::MatchReturn { where_clause, .. } = s else {
            panic!("expected MatchReturn");
        };
        let where_clause = where_clause.expect("WHERE clause");
        if let Expr::Binary { lhs, .. } = where_clause {
            if let Expr::Property { variable, path } = *lhs {
                assert_eq!(variable, "n");
                assert_eq!(path, vec!["_motif".to_string(), "foreshadow".to_string()]);
                return;
            }
        }
        panic!("expected Property{{variable, path}} on the LHS of WHERE");
    }

    #[test]
    fn parses_metadata_in_return() {
        let s = parse("MATCH (n) RETURN n._motif.foreshadow").unwrap();
        let Statement::MatchReturn { return_items, .. } = s else {
            panic!()
        };
        assert_eq!(
            return_items,
            vec![ReturnItem::Property {
                variable: "n".into(),
                path: vec!["_motif".into(), "foreshadow".into()],
            }]
        );
    }
}
