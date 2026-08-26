//! DekaScript parser entry point (Compiler v2).

use bumpalo::Bump;

use crate::ast::{Program, Span};
use crate::diagnostics::Diagnostic;

pub struct ParseResult<'a> {
    pub program: Option<Program<'a>>,
    pub errors: Vec<Diagnostic>,
}

pub fn parse<'a>(_source: &str, _arena: &'a Bump) -> ParseResult<'a> {
    ParseResult {
        program: Some(Program {
            statements: &[],
            span: Span::dummy(),
        }),
        errors: Vec::new(),
    }
}