//! DekaScript syntax crate (Compiler v2).
//!
//! Contains the DS-only lexer, parser, AST, and typechecker. No PHPX.

pub mod ast;
pub mod diagnostics;
pub mod lexer;
pub mod parse;
pub mod typeck;

pub use ast::*;
pub use diagnostics::{Diagnostic, Severity};
pub use lexer::Lexer;
pub use parse::{parse, ParseResult};
pub use typeck::{check_program, TypeError};