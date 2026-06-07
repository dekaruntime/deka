use php_rs::parser::ast::visitor::{Visitor, walk_expr, walk_stmt};
use php_rs::parser::ast::{Expr, ExprId, Program, Stmt, StmtId};
use php_rs::parser::span::Span as PhpxSpan;

use super::{ErrorKind, Severity, ValidationError};

/// Validate all `cql`/`query` expressions in a PHPX program.
///
/// For each `Expr::Cql` node:
/// 1. Run the Cypher subset parser on the raw body text
/// 2. Report syntax errors with positions mapped to the .phpx source
/// 3. Check that $param references resolve to variables in PHPX scope
pub fn validate_cypher(program: &Program, source: &str) -> Vec<ValidationError> {
    let mut validator = CypherValidator {
        source: source.as_bytes(),
        source_str: source,
        errors: Vec::new(),
        // Collect declared variables as we walk the AST
        declared_vars: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

struct CypherValidator<'a> {
    source: &'a [u8],
    source_str: &'a str,
    errors: Vec<ValidationError>,
    declared_vars: Vec<String>,
}

impl CypherValidator<'_> {
    fn push_error(
        &mut self,
        span: PhpxSpan,
        message: String,
        help: &str,
        suggestion: Option<String>,
    ) {
        if let Some(info) = span.line_info(self.source) {
            self.errors.push(ValidationError {
                kind: ErrorKind::CypherError,
                line: info.line,
                column: info.column,
                message,
                help_text: help.to_string(),
                suggestion,
                underline_length: (span.end - span.start).max(1),
                severity: Severity::Error,
            });
        }
    }

    fn push_warning(&mut self, span: PhpxSpan, message: String, help: &str) {
        if let Some(info) = span.line_info(self.source) {
            self.errors.push(ValidationError {
                kind: ErrorKind::CypherError,
                line: info.line,
                column: info.column,
                message,
                help_text: help.to_string(),
                suggestion: None,
                underline_length: (span.end - span.start).max(1),
                severity: Severity::Warning,
            });
        }
    }

    /// Find the closest match for a variable name among declared vars.
    fn suggest_variable(&self, name: &str) -> Option<String> {
        let mut best: Option<(&str, usize)> = None;
        for var in &self.declared_vars {
            let dist = levenshtein(name, var);
            if dist <= 3 {
                if best.is_none() || dist < best.unwrap().1 {
                    best = Some((var, dist));
                }
            }
        }
        best.map(|(s, _)| format!("${}", s))
    }

    fn validate_cql_expr(
        &mut self,
        cypher_span: PhpxSpan,
        params: &[php_rs::parser::ast::CqlParam],
    ) {
        // Get the raw Cypher text from the source
        let cypher_text = &self.source_str[cypher_span.start..cypher_span.end];
        let cypher_offset = cypher_span.start;

        // Run the Cypher parser
        let result = cypher::parse_cypher(cypher_text);

        // Convert Cypher parser errors to ValidationErrors with offset-adjusted spans
        for err in &result.errors {
            let phpx_span = PhpxSpan {
                start: cypher_offset + err.span.start,
                end: cypher_offset + err.span.end,
            };
            self.push_error(phpx_span, err.message.clone(), &err.help, None);
        }

        // Check that $param references resolve to PHPX variables in scope
        for param in params {
            let param_name = std::str::from_utf8(param.name).unwrap_or("");
            if !self.declared_vars.contains(&param_name.to_string()) {
                let suggestion = self.suggest_variable(param_name);
                let help = if suggestion.is_some() {
                    format!("Did you mean {}?", suggestion.as_ref().unwrap())
                } else {
                    "Declare the variable before the cql statement, or check the spelling."
                        .to_string()
                };
                self.push_error(
                    param.span,
                    format!("Variable ${} is not defined in this scope.", param_name),
                    &help,
                    suggestion,
                );
            }
        }
    }
}

impl<'ast> Visitor<'ast> for CypherValidator<'_> {
    fn visit_stmt(&mut self, stmt: StmtId<'ast>) {
        // Track variable declarations for scope checking
        match *stmt {
            Stmt::Expression { expr, .. } => {
                // Check for assignments: $var = expr
                if let Expr::Assign { var, .. } = expr {
                    if let Expr::Variable { name, .. } = var {
                        let var_name =
                            std::str::from_utf8(&self.source[name.start..name.end]).unwrap_or("");
                        // Strip the $ prefix if present
                        let clean = var_name.strip_prefix('$').unwrap_or(var_name);
                        if !clean.is_empty() {
                            self.declared_vars.push(clean.to_string());
                        }
                    }
                }
            }
            Stmt::Function { params, .. } => {
                // Function parameters are in scope
                for p in params.iter() {
                    let var_text =
                        std::str::from_utf8(&self.source[p.name.span.start..p.name.span.end])
                            .unwrap_or("");
                    let clean = var_text.strip_prefix('$').unwrap_or(var_text);
                    if !clean.is_empty() {
                        self.declared_vars.push(clean.to_string());
                    }
                }
            }
            _ => {}
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        if let Expr::Cql { cypher, params, .. } = expr {
            self.validate_cql_expr(*cypher, params);
        }
        walk_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler_api::compile_phpx;
    use bumpalo::Bump;

    #[test]
    fn valid_cql_produces_no_cypher_errors() {
        let source = r#"
$customer_id = 42;
$min_price = 10.00;
cql products = MATCH (c:Customer {id: $customer_id})-[:BOUGHT]->(p:Product) WHERE p.price > $min_price RETURN p.name, p.price;
"#;
        let arena = Bump::new();
        let result = compile_phpx(source, "test.phpx", &arena);
        let cypher_errors: Vec<_> = result
            .errors
            .iter()
            .filter(|e| e.kind == super::super::ErrorKind::CypherError)
            .collect();
        assert!(
            cypher_errors.is_empty(),
            "expected no cypher errors, got: {:?}",
            cypher_errors
        );
    }

    #[test]
    fn undefined_param_produces_error() {
        let source = r#"
$customer_id = 42;
cql results = MATCH (c:Customer) WHERE c.id = $cusomer_id RETURN c;
"#;
        let arena = Bump::new();
        let result = compile_phpx(source, "test.phpx", &arena);
        let cypher_errors: Vec<_> = result
            .errors
            .iter()
            .filter(|e| e.kind == super::super::ErrorKind::CypherError)
            .collect();
        assert_eq!(cypher_errors.len(), 1, "errors: {:?}", cypher_errors);
        assert!(cypher_errors[0].message.contains("cusomer_id"));
        // Should suggest $customer_id
        assert!(
            cypher_errors[0]
                .suggestion
                .as_ref()
                .is_some_and(|s| s.contains("customer_id")),
            "expected suggestion for $customer_id, got: {:?}",
            cypher_errors[0].suggestion
        );
    }

    #[test]
    fn invalid_cypher_syntax_produces_error() {
        let source = r#"
cql broken = MATCH (n:Person RETURN n;
"#;
        let arena = Bump::new();
        let result = compile_phpx(source, "test.phpx", &arena);
        let cypher_errors: Vec<_> = result
            .errors
            .iter()
            .filter(|e| e.kind == super::super::ErrorKind::CypherError)
            .collect();
        assert!(
            !cypher_errors.is_empty(),
            "expected cypher syntax error for unclosed node pattern"
        );
    }

    #[test]
    fn error_has_correct_line_and_column() {
        let source = "$x = 1\ncql q = MATCH (n) WHERE n.id = $typo RETURN n;\n";
        let arena = Bump::new();
        let result = compile_phpx(source, "test.phpx", &arena);
        let cypher_errors: Vec<_> = result
            .errors
            .iter()
            .filter(|e| e.kind == super::super::ErrorKind::CypherError)
            .collect();
        assert_eq!(cypher_errors.len(), 1);
        // $typo is on line 2
        assert_eq!(cypher_errors[0].line, 2, "error should be on line 2");
        assert!(cypher_errors[0].message.contains("typo"));
    }

    #[test]
    fn formatted_error_output_is_readable() {
        let source = "$customer_id = 42\ncql q = MATCH (c) WHERE c.id = $cusomer_id RETURN c;\n";
        let arena = Bump::new();
        let result = compile_phpx(source, "test.phpx", &arena);
        let cypher_errors: Vec<_> = result
            .errors
            .iter()
            .filter(|e| e.kind == super::super::ErrorKind::CypherError)
            .collect();
        assert_eq!(cypher_errors.len(), 1);

        let formatted =
            super::super::format_validation_error(source, "test.phpx", cypher_errors[0]);
        // Should contain the file path
        assert!(formatted.contains("test.phpx"), "should contain file path");
        // Should contain the error kind
        assert!(
            formatted.contains("Cypher Error"),
            "should contain 'Cypher Error'"
        );
        // Should contain the suggestion
        assert!(
            formatted.contains("customer_id"),
            "should contain suggestion 'customer_id'"
        );
    }

    #[test]
    fn cql_binding_registered_in_type_checker() {
        // After `cql q = ...`, $q should be usable without "unknown variable" type errors
        let source = r#"
function test() {
    $id = 42
    cql q = MATCH (n) WHERE n.id = $id RETURN n;
    $x = $q
}
"#;
        let arena = Bump::new();
        let result = compile_phpx(source, "test.phpx", &arena);
        let type_errors: Vec<_> = result
            .errors
            .iter()
            .filter(|e| e.message.contains("Unknown variable") && e.message.contains("$q"))
            .collect();
        assert!(
            type_errors.is_empty(),
            "cql binding $q should be recognized as a variable, got errors: {:?}",
            type_errors
        );
    }
}

/// Simple Levenshtein distance for "did you mean?" suggestions.
fn levenshtein(a: &str, b: &str) -> usize {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];

    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}
