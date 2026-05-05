//! Tokenizer for the v0.0.1 Cypher subset.
//!
//! The grammar is documented in `query/mod.rs`. This module turns a query
//! string into a stream of [`Token`]s with byte-offset spans for error
//! reporting. It is intentionally hand-rolled and minimal: no UTF-8
//! character-class trickery, no `regex` crate, no escape-sequence support
//! beyond `\\` and `\"` / `\'` and `\n` / `\t`.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Create,
    Match,
    Merge,
    Where,
    Return,
    Delete,
    Limit,
    And,
    Or,
    Not,
    True,
    False,
    Null,
    // Identifiers / literals
    Ident(String),
    Param(String),
    Integer(i64),
    Float(f64),
    String(String),
    // Punctuation / operators
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Create => write!(f, "CREATE"),
            Token::Match => write!(f, "MATCH"),
            Token::Merge => write!(f, "MERGE"),
            Token::Where => write!(f, "WHERE"),
            Token::Return => write!(f, "RETURN"),
            Token::Delete => write!(f, "DELETE"),
            Token::Limit => write!(f, "LIMIT"),
            Token::And => write!(f, "AND"),
            Token::Or => write!(f, "OR"),
            Token::Not => write!(f, "NOT"),
            Token::True => write!(f, "TRUE"),
            Token::False => write!(f, "FALSE"),
            Token::Null => write!(f, "NULL"),
            Token::Ident(s) => write!(f, "ident({s})"),
            Token::Param(s) => write!(f, "${s}"),
            Token::Integer(i) => write!(f, "{i}"),
            Token::Float(x) => write!(f, "{x}"),
            Token::String(s) => write!(f, "\"{s}\""),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::Comma => write!(f, ","),
            Token::Colon => write!(f, ":"),
            Token::Dot => write!(f, "."),
            Token::Eq => write!(f, "="),
            Token::NotEq => write!(f, "!="),
            Token::Lt => write!(f, "<"),
            Token::Gt => write!(f, ">"),
            Token::LtEq => write!(f, "<="),
            Token::GtEq => write!(f, ">="),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    pub token: Token,
    pub offset: usize,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LexError {
    #[error("unexpected character {ch:?} at offset {offset}")]
    Unexpected { ch: char, offset: usize },
    #[error("unterminated string starting at offset {offset}")]
    UnterminatedString { offset: usize },
    #[error("invalid escape sequence at offset {offset}")]
    InvalidEscape { offset: usize },
    #[error("invalid number literal at offset {offset}")]
    InvalidNumber { offset: usize },
}

pub fn lex(input: &str) -> Result<Vec<Spanned>, LexError> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i] as char;

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        let start = i;

        match c {
            '(' => {
                out.push(Spanned {
                    token: Token::LParen,
                    offset: start,
                });
                i += 1;
            }
            ')' => {
                out.push(Spanned {
                    token: Token::RParen,
                    offset: start,
                });
                i += 1;
            }
            '{' => {
                out.push(Spanned {
                    token: Token::LBrace,
                    offset: start,
                });
                i += 1;
            }
            '}' => {
                out.push(Spanned {
                    token: Token::RBrace,
                    offset: start,
                });
                i += 1;
            }
            ',' => {
                out.push(Spanned {
                    token: Token::Comma,
                    offset: start,
                });
                i += 1;
            }
            ':' => {
                out.push(Spanned {
                    token: Token::Colon,
                    offset: start,
                });
                i += 1;
            }
            '.' => {
                out.push(Spanned {
                    token: Token::Dot,
                    offset: start,
                });
                i += 1;
            }
            '=' => {
                // Accept `=` and `==` as equivalent; one fewer parser branch.
                i += 1;
                if i < bytes.len() && bytes[i] == b'=' {
                    i += 1;
                }
                out.push(Spanned {
                    token: Token::Eq,
                    offset: start,
                });
            }
            '!' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Spanned {
                        token: Token::NotEq,
                        offset: start,
                    });
                    i += 2;
                } else {
                    return Err(LexError::Unexpected {
                        ch: '!',
                        offset: start,
                    });
                }
            }
            '<' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Spanned {
                        token: Token::LtEq,
                        offset: start,
                    });
                    i += 2;
                } else {
                    out.push(Spanned {
                        token: Token::Lt,
                        offset: start,
                    });
                    i += 1;
                }
            }
            '>' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Spanned {
                        token: Token::GtEq,
                        offset: start,
                    });
                    i += 2;
                } else {
                    out.push(Spanned {
                        token: Token::Gt,
                        offset: start,
                    });
                    i += 1;
                }
            }
            '"' | '\'' => {
                let (s, end) = scan_string(input, i, c)?;
                out.push(Spanned {
                    token: Token::String(s),
                    offset: start,
                });
                i = end;
            }
            '$' => {
                i += 1;
                let name_start = i;
                while i < bytes.len() && is_ident_continue(bytes[i] as char) {
                    i += 1;
                }
                if i == name_start {
                    return Err(LexError::Unexpected {
                        ch: '$',
                        offset: start,
                    });
                }
                let name = input[name_start..i].to_owned();
                out.push(Spanned {
                    token: Token::Param(name),
                    offset: start,
                });
            }
            '-' | '0'..='9' => {
                let (tok, end) = scan_number(input, i)?;
                out.push(Spanned {
                    token: tok,
                    offset: start,
                });
                i = end;
            }
            c if is_ident_start(c) => {
                while i < bytes.len() && is_ident_continue(bytes[i] as char) {
                    i += 1;
                }
                let lex = &input[start..i];
                out.push(Spanned {
                    token: keyword_or_ident(lex),
                    offset: start,
                });
            }
            _ => {
                return Err(LexError::Unexpected {
                    ch: c,
                    offset: start,
                })
            }
        }
    }

    Ok(out)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn keyword_or_ident(s: &str) -> Token {
    let upper: String = s.chars().map(|c| c.to_ascii_uppercase()).collect();
    match upper.as_str() {
        "CREATE" => Token::Create,
        "MATCH" => Token::Match,
        "MERGE" => Token::Merge,
        "WHERE" => Token::Where,
        "RETURN" => Token::Return,
        "DELETE" => Token::Delete,
        "LIMIT" => Token::Limit,
        "AND" => Token::And,
        "OR" => Token::Or,
        "NOT" => Token::Not,
        "TRUE" => Token::True,
        "FALSE" => Token::False,
        "NULL" => Token::Null,
        _ => Token::Ident(s.to_owned()),
    }
}

fn scan_string(input: &str, start: usize, quote: char) -> Result<(String, usize), LexError> {
    let bytes = input.as_bytes();
    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == quote {
            return Ok((out, i + 1));
        }
        if c == '\\' {
            i += 1;
            if i >= bytes.len() {
                return Err(LexError::InvalidEscape { offset: i - 1 });
            }
            match bytes[i] as char {
                '\\' => out.push('\\'),
                '\'' => out.push('\''),
                '"' => out.push('"'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                _ => return Err(LexError::InvalidEscape { offset: i - 1 }),
            }
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    Err(LexError::UnterminatedString { offset: start })
}

fn scan_number(input: &str, start: usize) -> Result<(Token, usize), LexError> {
    let bytes = input.as_bytes();
    let mut i = start;
    if bytes[i] == b'-' {
        i += 1;
    }
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == int_start {
        return Err(LexError::InvalidNumber { offset: start });
    }

    let mut is_float = false;
    if i < bytes.len() && bytes[i] == b'.' {
        is_float = true;
        i += 1;
        let frac_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            return Err(LexError::InvalidNumber { offset: start });
        }
    }

    let lex = &input[start..i];
    if is_float {
        let v: f64 = lex
            .parse()
            .map_err(|_| LexError::InvalidNumber { offset: start })?;
        Ok((Token::Float(v), i))
    } else {
        let v: i64 = lex
            .parse()
            .map_err(|_| LexError::InvalidNumber { offset: start })?;
        Ok((Token::Integer(v), i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Token> {
        lex(s).unwrap().into_iter().map(|t| t.token).collect()
    }

    #[test]
    fn lexes_create_node() {
        assert_eq!(
            toks("CREATE (n:Person {age: 30})"),
            vec![
                Token::Create,
                Token::LParen,
                Token::Ident("n".into()),
                Token::Colon,
                Token::Ident("Person".into()),
                Token::LBrace,
                Token::Ident("age".into()),
                Token::Colon,
                Token::Integer(30),
                Token::RBrace,
                Token::RParen,
            ]
        );
    }

    #[test]
    fn lexes_match_with_param_and_string() {
        assert_eq!(
            toks("MATCH (n) WHERE n.name = 'Alice' RETURN n LIMIT 10"),
            vec![
                Token::Match,
                Token::LParen,
                Token::Ident("n".into()),
                Token::RParen,
                Token::Where,
                Token::Ident("n".into()),
                Token::Dot,
                Token::Ident("name".into()),
                Token::Eq,
                Token::String("Alice".into()),
                Token::Return,
                Token::Ident("n".into()),
                Token::Limit,
                Token::Integer(10),
            ]
        );
    }

    #[test]
    fn lexes_param_and_negative() {
        assert_eq!(
            toks("$x -42 1.5"),
            vec![
                Token::Param("x".into()),
                Token::Integer(-42),
                Token::Float(1.5),
            ]
        );
    }

    #[test]
    fn comparison_operators() {
        assert_eq!(
            toks("< <= > >= = == !="),
            vec![
                Token::Lt,
                Token::LtEq,
                Token::Gt,
                Token::GtEq,
                Token::Eq,
                Token::Eq,
                Token::NotEq,
            ]
        );
    }

    #[test]
    fn unterminated_string_errors() {
        let err = lex("'hi").unwrap_err();
        assert!(matches!(err, LexError::UnterminatedString { .. }));
    }

    #[test]
    fn keywords_are_case_insensitive() {
        assert_eq!(
            toks("create Match WhErE"),
            vec![Token::Create, Token::Match, Token::Where]
        );
    }
}
