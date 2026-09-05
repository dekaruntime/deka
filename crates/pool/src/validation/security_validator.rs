use runtime_core::security_policy::SecurityPolicy;
use swc_common::{FileName, SourceMap, Span, Spanned, sync::Lrc};
use swc_ecma_ast::*;
use swc_ecma_parser::{Parser, StringInput, Syntax, TsSyntax, lexer::Lexer};
use swc_ecma_visit::{Visit, VisitWith};

use super::error_formatter::format_validation_error;

const DYNAMIC_EVAL_OP: &str = "runtime.dynamic.eval";
const DYNAMIC_IMPORT_OP: &str = "runtime.dynamic.import";

pub fn validate_security_policy(
    source_code: &str,
    file_path: &str,
    policy: &SecurityPolicy,
) -> Result<(), String> {
    if source_code.trim().is_empty() {
        return Ok(());
    }

    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        FileName::Custom(file_path.to_string()).into(),
        source_code.to_string(),
    );
    let syntax = Syntax::Typescript(TsSyntax {
        tsx: file_path.ends_with(".tsx") || file_path.ends_with(".jsx"),
        decorators: false,
        dts: false,
        no_early_errors: true,
        disallow_ambiguous_jsx_like: false,
    });
    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let Ok(module) = parser.parse_module() else {
        // Existing compiler/runtime syntax diagnostics own parse failures.
        return Ok(());
    };

    let mut validator = SecurityVisitor {
        policy,
        violation: None,
        inside_allowed_process_chain: false,
    };
    module.visit_with(&mut validator);

    let Some(violation) = validator.violation else {
        return Ok(());
    };
    let start = cm.lookup_char_pos(violation.span.lo);
    let end = cm.lookup_char_pos(violation.span.hi);
    let underline_length = if start.line == end.line {
        end.col.0.saturating_sub(start.col.0).max(1)
    } else {
        1
    };

    Err(format_validation_error(
        source_code,
        file_path,
        "Security Policy Violation",
        start.line,
        start.col.0 + 1,
        &violation.message,
        &violation.help,
        underline_length,
    ))
}

struct Violation {
    span: Span,
    message: String,
    help: String,
}

struct SecurityVisitor<'a> {
    policy: &'a SecurityPolicy,
    violation: Option<Violation>,
    inside_allowed_process_chain: bool,
}

impl SecurityVisitor<'_> {
    fn prohibit_dynamic(&mut self, span: Span, operation: &str, op_id: &str) {
        if self.violation.is_none() && !self.policy.allows_dynamic() {
            self.violation = Some(Violation {
                span,
                message: format!("Prohibited dynamic operation: {operation} ({op_id})"),
                help: "Set `security.allow.dynamic` to true to permit dynamic code execution. An explicit `security.deny.dynamic` still takes precedence.".to_string(),
            });
        }
    }

    fn prohibit_ambient(&mut self, span: Span, operation: &str, help: String) {
        if self.violation.is_none() {
            self.violation = Some(Violation {
                span,
                message: format!("Prohibited ambient access: {operation}"),
                help,
            });
        }
    }

    fn validate_process_path(&mut self, span: Span, path: &[PathPart]) {
        if self.policy.allows_dynamic() {
            return;
        }

        let Some(PathPart::Known(first)) = path.first() else {
            self.prohibit_ambient(
                span,
                "computed process access",
                "Use `process.env.NAME` and list NAME in `security.allow.env`, or explicitly allow dynamic access.".to_string(),
            );
            return;
        };
        if first != "env" {
            self.prohibit_ambient(
                span,
                "process global",
                "Only manifest-allowed `process.env` names are available without `security.allow.dynamic`.".to_string(),
            );
            return;
        }

        match path.get(1) {
            None if self.policy.allows_any_env() => {}
            None => self.prohibit_ambient(
                span,
                "process.env",
                "Add explicit names to `security.allow.env` before accessing `process.env`.".to_string(),
            ),
            Some(PathPart::Known(name)) if self.policy.allows_env(name) => {}
            Some(PathPart::Known(name)) => self.prohibit_ambient(
                span,
                &format!("process.env.{name}"),
                format!("Add `{name}` to `security.allow.env` and ensure it is not present in `security.deny.env`."),
            ),
            Some(PathPart::Computed) if self.policy.allows_unrestricted_env() => {}
            Some(PathPart::Computed) => self.prohibit_ambient(
                span,
                "computed process.env access",
                "Computed environment access requires unrestricted `security.allow.env` with no deny rules; prefer a statically named allowlisted variable.".to_string(),
            ),
        }
    }
}

impl Visit for SecurityVisitor<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
        if self.violation.is_some() {
            return;
        }

        if !self.policy.allows_dynamic() {
            match expr {
                Expr::Ident(ident) if ident.sym == *"eval" => {
                    self.prohibit_dynamic(ident.span, "eval", DYNAMIC_EVAL_OP);
                    return;
                }
                Expr::Ident(ident) if ident.sym == *"Function" => {
                    self.prohibit_dynamic(ident.span, "Function constructor", DYNAMIC_EVAL_OP);
                    return;
                }
                Expr::Ident(ident)
                    if ident.sym == *"global" && !self.inside_allowed_process_chain =>
                {
                    self.prohibit_ambient(
                        ident.span,
                        "global",
                        "Use an explicit runtime API, or enable `security.allow.dynamic` for ambient global access.".to_string(),
                    );
                    return;
                }
                _ => {}
            }
        }

        if let Some(path) = static_path(expr) {
            if is_dynamic_member(&path, "eval") {
                self.prohibit_dynamic(expr.span(), "eval", DYNAMIC_EVAL_OP);
                return;
            }
            if is_dynamic_member(&path, "Function") {
                self.prohibit_dynamic(expr.span(), "Function constructor", DYNAMIC_EVAL_OP);
                return;
            }
            if !self.inside_allowed_process_chain
                && let Some(process_path) = process_path(&path)
            {
                self.validate_process_path(expr.span(), process_path);
                if self.violation.is_some() {
                    return;
                }
                self.inside_allowed_process_chain = true;
                expr.visit_children_with(self);
                self.inside_allowed_process_chain = false;
                return;
            }
            if is_computed_global_path(&path) && !uses_internal_host_symbol(expr) {
                self.prohibit_dynamic(expr.span(), "computed global access", DYNAMIC_EVAL_OP);
                return;
            }
        } else if matches!(expr, Expr::Member(member) if is_global_root(&member.obj)) {
            self.prohibit_dynamic(expr.span(), "computed global access", DYNAMIC_EVAL_OP);
            return;
        }

        expr.visit_children_with(self);
    }

    fn visit_call_expr(&mut self, call: &CallExpr) {
        if self.violation.is_some() {
            return;
        }
        if matches!(call.callee, Callee::Import(_)) {
            self.prohibit_dynamic(call.span, "dynamic import()", DYNAMIC_IMPORT_OP);
            return;
        }
        call.visit_children_with(self);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PathPart {
    Known(String),
    Computed,
}

fn static_path(expr: &Expr) -> Option<Vec<PathPart>> {
    match transparent_expr(expr) {
        Expr::Ident(ident) => Some(vec![PathPart::Known(ident.sym.to_string())]),
        Expr::Member(member) => {
            let mut path = static_path(&member.obj)?;
            path.push(match &member.prop {
                MemberProp::Ident(ident) => PathPart::Known(ident.sym.to_string()),
                MemberProp::PrivateName(_) => return None,
                MemberProp::Computed(computed) => match transparent_expr(&computed.expr) {
                    Expr::Lit(Lit::Str(value)) => {
                        PathPart::Known(value.value.to_string_lossy().into_owned())
                    }
                    _ => PathPart::Computed,
                },
            });
            Some(path)
        }
        _ => None,
    }
}

fn transparent_expr(mut expr: &Expr) -> &Expr {
    loop {
        expr = match expr {
            Expr::Paren(paren) => &paren.expr,
            Expr::TsAs(as_expr) => &as_expr.expr,
            Expr::TsTypeAssertion(assertion) => &assertion.expr,
            Expr::TsNonNull(non_null) => &non_null.expr,
            Expr::Seq(seq) => match seq.exprs.last() {
                Some(last) => last,
                None => return expr,
            },
            _ => return expr,
        };
    }
}

fn is_dynamic_member(path: &[PathPart], name: &str) -> bool {
    matches!(path, [PathPart::Known(root), rest @ ..]
        if (root == "globalThis" || root == "global")
            && matches!(rest.first(), Some(PathPart::Known(member)) if member == name))
}

fn is_computed_global_path(path: &[PathPart]) -> bool {
    matches!(path, [PathPart::Known(root), PathPart::Computed, ..]
        if root == "globalThis" || root == "global")
}

fn uses_internal_host_symbol(expr: &Expr) -> bool {
    let Expr::Member(member) = transparent_expr(expr) else {
        return false;
    };
    if is_global_root(&member.obj) {
        let MemberProp::Computed(computed) = &member.prop else {
            return false;
        };
        return is_internal_host_symbol_call(&computed.expr);
    }
    uses_internal_host_symbol(&member.obj)
}

fn is_internal_host_symbol_call(expr: &Expr) -> bool {
    let Expr::Call(call) = transparent_expr(expr) else {
        return false;
    };
    let Callee::Expr(callee) = &call.callee else {
        return false;
    };
    let Some(path) = static_path(callee) else {
        return false;
    };
    if !matches!(path.as_slice(), [PathPart::Known(symbol), PathPart::Known(method)] if symbol == "Symbol" && method == "for")
    {
        return false;
    }
    matches!(call.args.as_slice(), [arg]
        if matches!(transparent_expr(&arg.expr), Expr::Lit(Lit::Str(value)) if value.value == *"deka.host.internal"))
}

fn process_path(path: &[PathPart]) -> Option<&[PathPart]> {
    match path {
        [PathPart::Known(root), rest @ ..] if root == "process" => Some(rest),
        [PathPart::Known(root), PathPart::Known(process), rest @ ..]
            if (root == "globalThis" || root == "global") && process == "process" =>
        {
            Some(rest)
        }
        _ => None,
    }
}

fn is_global_root(expr: &Expr) -> bool {
    matches!(transparent_expr(expr), Expr::Ident(ident) if ident.sym == *"globalThis" || ident.sym == *"global")
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_core::security_policy::{RuleList, SecurityPolicy};

    fn validate(source: &str, policy: &SecurityPolicy) -> Result<(), String> {
        validate_security_policy(source, "handler.js", policy)
    }

    fn dynamic_policy() -> SecurityPolicy {
        let mut policy = SecurityPolicy::default();
        policy.allow.dynamic = true;
        policy
    }

    #[test]
    fn blocks_eval_and_function_ast_forms_by_default() {
        for (source, operation) in [
            ("eval('1 + 1')", "eval"),
            ("(0, eval)('1 + 1')", "eval"),
            ("globalThis['eval']('1 + 1')", "eval"),
            ("new Function('return 7')()", "Function constructor"),
            ("globalThis.Function('return 7')()", "Function constructor"),
        ] {
            let error = validate(source, &SecurityPolicy::default()).unwrap_err();
            assert!(error.contains(operation), "source={source}\n{error}");
            assert!(error.contains(DYNAMIC_EVAL_OP), "source={source}\n{error}");
        }
    }

    #[test]
    fn blocks_dynamic_import_by_default() {
        let error = validate("import('./plugin.js')", &SecurityPolicy::default()).unwrap_err();
        assert!(error.contains("dynamic import()"), "{error}");
        assert!(error.contains(DYNAMIC_IMPORT_OP), "{error}");
    }

    #[test]
    fn allows_dynamic_operations_when_dynamic_is_effectively_allowed() {
        for source in [
            "eval('1 + 1')",
            "new Function('return 7')()",
            "import('./plugin.js')",
            "globalThis[operation]()",
            "global.process",
        ] {
            validate(source, &dynamic_policy()).unwrap_or_else(|error| {
                panic!("source={source}\n{error}");
            });
        }
    }

    #[test]
    fn explicit_dynamic_deny_overrides_allow() {
        let mut policy = dynamic_policy();
        policy.deny.dynamic = true;
        let error = validate("eval('1 + 1')", &policy).unwrap_err();
        assert!(error.contains(DYNAMIC_EVAL_OP), "{error}");
    }

    #[test]
    fn process_env_access_obeys_manifest_allow_and_deny_lists() {
        let mut policy = SecurityPolicy::default();
        policy.allow.env = RuleList::List(vec!["PUBLIC_KEY".to_string(), "BLOCKED".to_string()]);
        policy.deny.env = RuleList::List(vec!["BLOCKED".to_string()]);

        validate("process.env.PUBLIC_KEY", &policy).unwrap();
        validate("globalThis.process.env['PUBLIC_KEY']", &policy).unwrap();
        validate("global.process.env.PUBLIC_KEY", &policy).unwrap();

        for source in [
            "process.env.SECRET_KEY",
            "globalThis.process.env.BLOCKED",
            "process.env[name]",
            "typeof globalThis.process",
        ] {
            let error = validate(source, &policy).unwrap_err();
            assert!(
                error.contains("Prohibited ambient access"),
                "source={source}\n{error}"
            );
        }
    }

    #[test]
    fn unrestricted_env_policy_allows_computed_env_access() {
        let mut policy = SecurityPolicy::default();
        policy.allow.env = RuleList::All;
        validate("process.env[name]", &policy).unwrap();

        policy.deny.env = RuleList::List(vec!["SECRET".to_string()]);
        validate("process.env[name]", &policy).unwrap_err();
    }

    #[test]
    fn only_the_existing_internal_symbol_bypasses_computed_global_rejection() {
        let policy = SecurityPolicy::default();
        validate("globalThis.app", &policy).unwrap();
        validate("globalThis[Symbol.for('deka.host.internal')].ops", &policy).unwrap();
        for source in ["globalThis[operation]()", "globalThis[Symbol.for('other')]"] {
            let error = validate(source, &policy).unwrap_err();
            assert!(error.contains("Prohibited"), "{source}\n{error}");
        }
    }
}
