//! DekaScript typechecker (Compiler v2).

use crate::ast::Program;
use crate::diagnostics::Diagnostic;

#[derive(Debug)]
pub struct TypeError {
    pub message: String,
}

pub struct TypeckResult<'a> {
    pub program: &'a Program<'a>,
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
}

pub fn check_program<'a>(program: &'a Program<'a>, _source: &str) -> TypeckResult<'a> {
    TypeckResult {
        program,
        errors: Vec::new(),
        warnings: Vec::new(),
    }
}