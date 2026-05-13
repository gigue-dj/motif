//! Query layer for the v0.0.1 Cypher subset.
//!
//! ## Supported grammar
//!
//! ```text
//! statement = create | merge | match-return | match-delete
//!
//! create        = "CREATE" node-pattern
//! merge         = "MERGE" node-pattern
//! match-return  = "MATCH" node-pattern ("WHERE" expr)?
//!                 "RETURN" return-item ("," return-item)*
//!                 ("LIMIT" integer)?
//! match-delete  = "MATCH" node-pattern ("WHERE" expr)? "DELETE" ident
//!
//! node-pattern  = "(" ident (":" ident)? property-map? ")"
//! property-map  = "{" (ident ":" expr ("," ident ":" expr)*)? "}"
//! return-item   = ident | ident "." ident
//!
//! expr          = or-expr
//! or-expr       = and-expr ("OR" and-expr)*
//! and-expr      = not-expr ("AND" not-expr)*
//! not-expr      = "NOT" not-expr | comparison
//! comparison    = primary (op primary)?
//! op            = "=" | "==" | "!=" | "<" | ">" | "<=" | ">="
//! primary       = literal | parameter | "id" "(" ident ")"
//!               | ident "." ident | "(" expr ")"
//! literal       = integer | float | string | "TRUE" | "FALSE" | "NULL"
//! parameter     = "$" ident
//! ```
//!
//! ## v0.0.1 limitations
//!
//! - Single bound variable per statement.
//! - Edges are not addressable from queries — only via the engine API.
//!   Edge query support lands in alpha.5.
//! - `CREATE` and `MERGE` require an explicit `id` string property; the
//!   engine does not assign synthetic ids.
//! - The only built-in function is `id(n)`.
//! - `MERGE` is no-op-on-hit, not "match-or-create-with-this-pattern" — it
//!   does not update the existing node.

pub mod ast;
pub mod interpreter;
pub mod lexer;
pub mod parser;
pub mod result;

pub use ast::Statement;
pub use interpreter::{execute, InterpretError, Params};
pub use lexer::LexError;
pub use parser::ParseError;
pub use result::{QueryResult, ResultCell};

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("execution error: {0}")]
    Interpret(#[from] InterpretError),
}

impl From<LexError> for QueryError {
    fn from(e: LexError) -> Self {
        QueryError::Parse(ParseError::Lex(e))
    }
}

pub fn parse(src: &str) -> Result<Statement, QueryError> {
    let tokens = lexer::lex(src)?;
    let stmt = parser::parse_tokens(&tokens)?;
    Ok(stmt)
}
