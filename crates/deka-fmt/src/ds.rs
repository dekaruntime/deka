//! DekaScript source formatter.
//!
//! v2 is AST-aware: it parses the source with `php-rs` in DekaScript mode and
//! pretty-prints a canonical layout. If the source cannot be parsed, it is
//! returned unchanged so the formatter is safe to run on incomplete code.

use php_rs::parser::ast::*;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use php_rs::parser::span::Span;

/// Format a DekaScript source string.
///
/// Parsed programs are pretty-printed with 2-space indentation and an
/// 80-character soft line-width target. If parsing fails, the original source
/// is returned unchanged.
pub fn format_ds(source: &str) -> Result<String, String> {
    let arena = bumpalo::Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        // Formatter is conservative: don't try to repair broken code.
        return Ok(source.to_string());
    }
    let mut fmt = Formatter::new(source);
    fmt.fmt_program(&program);
    fmt.finish()
}

struct Formatter<'src> {
    source: &'src str,
    out: String,
    indent: usize,
    /// Tracks whether the last character written was a newline so we can emit
    /// indentation before the next non-whitespace token.
    at_line_start: bool,
}

impl<'src> Formatter<'src> {
    fn new(source: &'src str) -> Self {
        Self {
            source,
            out: String::new(),
            indent: 0,
            at_line_start: true,
        }
    }

    fn finish(mut self) -> Result<String, String> {
        // Trim trailing blank lines, then ensure exactly one trailing newline.
        while self.out.ends_with("\n\n") {
            self.out.pop();
        }
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        Ok(self.out)
    }

    // --- output helpers ----------------------------------------------------

    fn write(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if self.at_line_start && s != "\n" {
            for _ in 0..self.indent {
                self.out.push_str("  ");
            }
            self.at_line_start = false;
        }
        self.out.push_str(s);
        if s.ends_with('\n') {
            self.at_line_start = true;
        }
    }

    fn newline(&mut self) {
        self.out.push('\n');
        self.at_line_start = true;
    }

    fn indented<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.indent += 1;
        let result = f(self);
        self.indent -= 1;
        result
    }

    fn token_text(&self, token: &php_rs::parser::lexer::token::Token) -> String {
        std::str::from_utf8(token.text(self.source.as_bytes()))
            .unwrap_or("")
            .to_string()
    }

    fn span_text(&self, span: php_rs::parser::span::Span) -> String {
        std::str::from_utf8(span.as_str(self.source.as_bytes()))
            .unwrap_or("")
            .to_string()
    }

    fn name_text(&self, name: &Name<'_>) -> String {
        name.parts
            .iter()
            .map(|t| self.token_text(t))
            .collect::<Vec<_>>()
            .join("")
    }

    fn stmt_span(&self, stmt: &Stmt<'_>) -> Span {
        match stmt {
            Stmt::Echo { span, .. } => *span,
            Stmt::Return { span, .. } => *span,
            Stmt::If { span, .. } => *span,
            Stmt::While { span, .. } => *span,
            Stmt::DoWhile { span, .. } => *span,
            Stmt::For { span, .. } => *span,
            Stmt::Foreach { span, .. } => *span,
            Stmt::Block { span, .. } => *span,
            Stmt::Function { span, .. } => *span,
            Stmt::ReceiverMethod { span, .. } => *span,
            Stmt::TypeAlias { span, .. } => *span,
            Stmt::Class { span, .. } => *span,
            Stmt::Interface { span, .. } => *span,
            Stmt::Trait { span, .. } => *span,
            Stmt::Enum { span, .. } => *span,
            Stmt::Namespace { span, .. } => *span,
            Stmt::Use { span, .. } => *span,
            Stmt::Import { span, .. } => *span,
            Stmt::Export { span, .. } => *span,
            Stmt::Switch { span, .. } => *span,
            Stmt::Try { span, .. } => *span,
            Stmt::Throw { span, .. } => *span,
            Stmt::Const { span, .. } => *span,
            Stmt::Break { span, .. } => *span,
            Stmt::Continue { span, .. } => *span,
            Stmt::Global { span, .. } => *span,
            Stmt::Static { span, .. } => *span,
            Stmt::Unset { span, .. } => *span,
            Stmt::Expression { span, .. } => *span,
            Stmt::InlineHtml { span, .. } => *span,
            Stmt::Nop { span, .. } => *span,
            Stmt::Label { span, .. } => *span,
            Stmt::Goto { span, .. } => *span,
            Stmt::Error { span, .. } => *span,
            Stmt::Declare { span, .. } => *span,
            Stmt::HaltCompiler { span, .. } => *span,
        }
    }

    fn expr_span(&self, expr: &Expr<'_>) -> Span {
        match expr {
            Expr::Assign { span, .. } => *span,
            Expr::AssignRef { span, .. } => *span,
            Expr::AssignOp { span, .. } => *span,
            Expr::Binary { span, .. } => *span,
            Expr::Unary { span, .. } => *span,
            Expr::Call { span, .. } => *span,
            Expr::Array { span, .. } => *span,
            Expr::ObjectLiteral { span, .. } => *span,
            Expr::JsxElement { span, .. } => *span,
            Expr::JsxFragment { span, .. } => *span,
            Expr::StructLiteral { span, .. } => *span,
            Expr::ArrayDimFetch { span, .. } => *span,
            Expr::DotAccess { span, .. } => *span,
            Expr::PropertyFetch { span, .. } => *span,
            Expr::MethodCall { span, .. } => *span,
            Expr::StaticCall { span, .. } => *span,
            Expr::ClassConstFetch { span, .. } => *span,
            Expr::New { span, .. } => *span,
            Expr::Variable { span, .. } => *span,
            Expr::IndirectVariable { span, .. } => *span,
            Expr::Integer { span, .. } => *span,
            Expr::Float { span, .. } => *span,
            Expr::BigInt { span, .. } => *span,
            Expr::Boolean { span, .. } => *span,
            Expr::Null { span, .. } => *span,
            Expr::String { span, .. } => *span,
            Expr::InterpolatedString { span, .. } => *span,
            Expr::ShellExec { span, .. } => *span,
            Expr::Include { span, .. } => *span,
            Expr::MagicConst { span, .. } => *span,
            Expr::PostInc { span, .. } => *span,
            Expr::PostDec { span, .. } => *span,
            Expr::Ternary { span, .. } => *span,
            Expr::Match { span, .. } => *span,
            Expr::AnonymousClass { span, .. } => *span,
            Expr::Print { span, .. } => *span,
            Expr::Yield { span, .. } => *span,
            Expr::Cast { span, .. } => *span,
            Expr::Empty { span, .. } => *span,
            Expr::Isset { span, .. } => *span,
            Expr::Eval { span, .. } => *span,
            Expr::Await { span, .. } => *span,
            Expr::Die { span, .. } => *span,
            Expr::Exit { span, .. } => *span,
            Expr::Closure { span, .. } => *span,
            Expr::ArrowFunction { span, .. } => *span,
            Expr::Clone { span, .. } => *span,
            Expr::NullsafePropertyFetch { span, .. } => *span,
            Expr::NullsafeMethodCall { span, .. } => *span,
            Expr::VariadicPlaceholder { span, .. } => *span,
            Expr::Spread { span, .. } => *span,
            Expr::Unsafe { span, .. } => *span,
            Expr::Bridge { span, .. } => *span,
            Expr::Cql { span, .. } => *span,
            Expr::Error { span, .. } => *span,
        }
    }

    fn stmt_start_line(&self, stmt: &Stmt<'_>) -> usize {
        self.stmt_span(stmt)
            .line_info(self.source.as_bytes())
            .map(|li| li.line)
            .unwrap_or(0)
    }

    fn emit_stmt_separator(&mut self, prev_start_line: usize, next_start_line: usize) {
        if next_start_line > prev_start_line + 1 {
            self.write("\n\n");
        } else {
            self.newline();
        }
    }

    // --- program & statements ----------------------------------------------

    fn fmt_program(&mut self, program: &Program<'_>) {
        let mut first = true;
        let mut prev_line: Option<usize> = None;
        for stmt in program.statements {
            if matches!(stmt, Stmt::Nop { .. }) {
                continue;
            }
            let next_line = self.stmt_start_line(stmt);
            if !first {
                self.emit_stmt_separator(prev_line.unwrap_or(0), next_line);
            }
            first = false;
            self.fmt_stmt(stmt);
            prev_line = Some(next_line);
        }
    }

    fn fmt_stmt(&mut self, stmt: &Stmt<'_>) {
        match stmt {
            Stmt::Echo { exprs, .. } => {
                self.write("print(");
                self.fmt_expr_list(exprs, ", ");
                self.write(")");
            }
            Stmt::Return { expr: None, .. } => self.write("return"),
            Stmt::Return { expr: Some(expr), .. } => {
                self.write("return ");
                self.fmt_expr(expr);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.write("if (");
                self.fmt_expr(condition);
                self.write(") ");
                self.fmt_block(then_block);
                if let Some(else_block) = else_block {
                    self.write(" else ");
                    if else_block.len() == 1 && matches!(else_block[0], Stmt::If { .. }) {
                        self.fmt_stmt(&else_block[0]);
                    } else {
                        self.fmt_block(else_block);
                    }
                }
            }
            Stmt::While { condition, body, .. } => {
                self.write("while (");
                self.fmt_expr(condition);
                self.write(") ");
                self.fmt_block(body);
            }
            Stmt::DoWhile { body, condition, .. } => {
                self.write("do ");
                self.fmt_block(body);
                self.write(" while (");
                self.fmt_expr(condition);
                self.write(");");
            }
            Stmt::For {
                init,
                condition,
                loop_expr,
                body,
                ..
            } => {
                self.write("for (");
                self.fmt_expr_list(init, ", ");
                self.write("; ");
                self.fmt_expr_list(condition, ", ");
                self.write("; ");
                self.fmt_expr_list(loop_expr, ", ");
                self.write(") ");
                self.fmt_block(body);
            }
            Stmt::Foreach {
                expr,
                key_var,
                value_var,
                body,
                ..
            } => {
                self.write("for (const ");
                if let Some(key) = key_var {
                    self.fmt_expr(key);
                    self.write(" of ");
                    self.fmt_expr(expr);
                } else {
                    self.fmt_expr(value_var);
                    self.write(" of ");
                    self.fmt_expr(expr);
                }
                self.write(") ");
                self.fmt_block(body);
            }
            Stmt::Block { statements, .. } => self.fmt_block(statements),
            Stmt::Function {
                name,
                is_async,
                type_params,
                params,
                return_type,
                body,
                ..
            } => {
                self.fmt_fn_sig(*is_async, Some(name), type_params, params, *return_type);
                self.write(" ");
                self.fmt_block(body);
            }
            Stmt::ReceiverMethod {
                name,
                is_async,
                receiver,
                params,
                return_type,
                body,
                ..
            } => {
                self.fmt_receiver_method_sig(
                    *is_async,
                    name,
                    receiver,
                    params,
                    *return_type,
                );
                self.write(" ");
                self.fmt_block(body);
            }
            Stmt::TypeAlias {
                name,
                type_params,
                ty,
                ..
            } => {
                self.write("type ");
                self.write(&self.token_text(name));
                if !type_params.is_empty() {
                    self.write("<");
                    self.fmt_type_param_list(type_params);
                    self.write(">");
                }
                self.write(" = ");
                self.fmt_type(ty);
            }
            Stmt::Class {
                kind,
                name,
                extends,
                implements,
                members,
                ..
            } => {
                match kind {
                    ClassKind::Struct => self.write("struct "),
                    ClassKind::Class => self.write("class "),
                }
                self.write(&self.token_text(name));
                if let Some(base) = extends {
                    self.write(" : ");
                    self.write(&self.name_text(base));
                }
                if !implements.is_empty() {
                    self.write(" implements ");
                    self.write(
                        &implements
                            .iter()
                            .map(|n| self.name_text(n))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                self.write(" {");
                if !members.is_empty() {
                    self.newline();
                    self.indented(|this| {
                        this.fmt_class_members(members);
                        this.newline();
                    });
                    self.write("}");
                } else {
                    self.write("}");
                }
            }
            Stmt::Interface {
                name, members, ..
            } => {
                self.write("interface ");
                self.write(&self.token_text(name));
                self.write(" {");
                if !members.is_empty() {
                    self.newline();
                    self.indented(|this| {
                        this.fmt_class_members(members);
                        this.newline();
                    });
                    self.write("}");
                } else {
                    self.write("}");
                }
            }
            Stmt::Enum {
                name,
                type_params,
                backed_type,
                members,
                ..
            } => {
                self.write("enum ");
                self.write(&self.token_text(name));
                if !type_params.is_empty() {
                    self.write("<");
                    self.fmt_type_param_list(type_params);
                    self.write(">");
                }
                if let Some(ty) = backed_type {
                    self.write(": ");
                    self.fmt_type(ty);
                }
                self.write(" {");
                if !members.is_empty() {
                    self.newline();
                    self.indented(|this| {
                        this.fmt_class_members(members);
                        this.newline();
                    });
                    self.write("}");
                } else {
                    self.write("}");
                }
            }
            Stmt::Namespace { name, body, .. } => {
                self.write("namespace ");
                if let Some(name) = name {
                    self.write(&self.name_text(name));
                }
                if let Some(body) = body {
                    self.write(" {");
                    self.newline();
                    self.indented(|this| this.fmt_program(&Program {
                        statements: body,
                        errors: &[],
                        span: Span::default(),
                    }));
                    self.write("}");
                } else {
                    self.write(";");
                }
            }
            Stmt::Use { uses, .. } => {
                self.write("use ");
                let parts: Vec<String> = uses
                    .iter()
                    .map(|u| {
                        let mut s = self.name_text(&u.name);
                        if let Some(alias) = u.alias {
                            s.push_str(" as ");
                            s.push_str(&self.token_text(alias));
                        }
                        s
                    })
                    .collect();
                self.write(&parts.join(", "));
            }
            Stmt::Switch { condition, cases, .. } => {
                self.write("switch (");
                self.fmt_expr(condition);
                self.write(") {");
                self.newline();
                self.indented(|this| {
                    for case in *cases {
                        this.fmt_case(case);
                    }
                });
                self.write("}");
            }
            Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
                self.write("try ");
                self.fmt_block(body);
                for catch in *catches {
                    self.write(" catch (");
                    self.write(
                        &catch
                            .types
                            .iter()
                            .map(|t| self.name_text(t))
                            .collect::<Vec<_>>()
                            .join(" | "),
                    );
                    if let Some(var) = catch.var {
                        self.write(" ");
                        self.write(&self.token_text(var));
                    }
                    self.write(") ");
                    self.fmt_block(catch.body);
                }
                if let Some(finally) = finally {
                    self.write(" finally ");
                    self.fmt_block(finally);
                }
            }
            Stmt::Throw { expr, .. } => {
                self.write("throw ");
                self.fmt_expr(expr);
            }
            Stmt::Const { consts, .. } => {
                self.write("const ");
                let parts: Vec<String> = consts
                    .iter()
                    .map(|c| {
                        let mut s = self.token_text(c.name).to_string();
                        // A declared binding type is part of the source, not a
                        // hint: dropping it here would silently delete what the
                        // author wrote, and under "fmt is compile" that edit
                        // becomes canonical.
                        if let Some(ty) = c.ty {
                            s.push_str(": ");
                            s.push_str(&self.type_to_string(ty));
                        }
                        s.push_str(" = ");
                        s.push_str(&self.expr_to_string(&c.value));
                        s
                    })
                    .collect();
                self.write(&parts.join(", "));
            }
            Stmt::Static { vars, .. } => {
                self.write("let ");
                let parts: Vec<String> = vars
                    .iter()
                    .map(|v| {
                        let mut s = self.expr_to_string(v.var);
                        if let Some(ty) = v.ty {
                            s.push_str(": ");
                            s.push_str(&self.type_to_string(ty));
                        }
                        if let Some(default) = v.default {
                            s.push_str(" = ");
                            s.push_str(&self.expr_to_string(default));
                        }
                        s
                    })
                    .collect();
                self.write(&parts.join(", "));
            }
            Stmt::Break { level: None, .. } => self.write("break"),
            Stmt::Break { level: Some(level), .. } => {
                self.write("break ");
                self.fmt_expr(level);
            }
            Stmt::Continue { level: None, .. } => self.write("continue"),
            Stmt::Continue { level: Some(level), .. } => {
                self.write("continue ");
                self.fmt_expr(level);
            }
            Stmt::Global { vars, .. } => {
                self.write("global ");
                self.fmt_expr_list(vars, ", ");
            }
            Stmt::Unset { vars, .. } => {
                self.write("unset(");
                self.fmt_expr_list(vars, ", ");
                self.write(")");
            }
            Stmt::Expression { expr, .. } => {
                self.fmt_expr(expr);
            }
            Stmt::Label { name, .. } => {
                self.write(&self.token_text(name));
                self.write(":");
            }
            Stmt::Goto { label, .. } => {
                self.write("goto ");
                self.write(&self.token_text(label));
            }
            Stmt::InlineHtml { value, .. } => {
                self.write("<?=");
                self.write(std::str::from_utf8(value).unwrap_or(""));
                self.write("?>");
            }
            Stmt::Declare { declares, body, .. } => {
                self.write("declare(");
                let parts: Vec<String> = declares
                    .iter()
                    .map(|d| {
                        let mut s = self.token_text(d.key).to_string();
                        s.push_str(" = ");
                        s.push_str(&self.expr_to_string(&d.value));
                        s
                    })
                    .collect();
                self.write(&parts.join(", "));
                self.write(")");
                if body.is_empty() {
                    self.write(";");
                } else {
                    self.write(" {");
                    self.newline();
                    self.indented(|this| {
                        for stmt in *body {
                            this.fmt_stmt(stmt);
                            this.newline();
                        }
                    });
                    self.write("}");
                }
            }
            Stmt::HaltCompiler { .. } => self.write("__halt_compiler();"),
            Stmt::Nop { .. } => {}
            Stmt::Error { .. } => {}
            Stmt::Import { span, .. } | Stmt::Export { span, .. } => {
                // Phase 1: preserve original source text for import/export statements.
                // Full formatter support will follow once the AST-based module system
                // stabilizes.
                let text = std::str::from_utf8(span.as_str(self.source.as_bytes())).unwrap_or("");
                self.write(text);
            }
            Stmt::Trait { .. } => {
                // DekaScript rejects traits, but we still emit a placeholder
                // so the formatter does not panic on edge-case input.
                self.write("/* trait omitted */");
            }
        }
    }

    fn fmt_block(&mut self, stmts: &[StmtId<'_>]) {
        self.write("{");
        if stmts.is_empty() {
            self.write("}");
            return;
        }
        self.newline();
        self.indented(|this| {
            let mut first = true;
            let mut prev_line: Option<usize> = None;
            for stmt in stmts {
                if matches!(stmt, Stmt::Nop { .. }) {
                    continue;
                }
                let next_line = this.stmt_start_line(stmt);
                if !first {
                    this.emit_stmt_separator(prev_line.unwrap_or(0), next_line);
                }
                first = false;
                this.fmt_stmt(stmt);
                prev_line = Some(next_line);
            }
        });
        self.newline();
        self.write("}");
    }

    fn fmt_case(&mut self, case: &Case<'_>) {
        match case.condition {
            Some(expr) => {
                self.write("case ");
                self.fmt_expr(expr);
                self.write(":");
            }
            None => self.write("default:"),
        }
        if !case.body.is_empty() {
            self.newline();
            self.indented(|this| {
                let mut first = true;
                for stmt in case.body {
                    if !first {
                        this.newline();
                    }
                    first = false;
                    this.fmt_stmt(stmt);
                }
            });
            self.newline();
        }
    }

    // --- class / interface / enum members ----------------------------------

    fn fmt_class_members(&mut self, members: &[ClassMember<'_>]) {
        let mut first = true;
        for member in members {
            if !first {
                self.newline();
            }
            first = false;
            self.fmt_class_member(member);
        }
    }

    fn fmt_class_member(&mut self, member: &ClassMember<'_>) {
        match member {
            ClassMember::Property {
                modifiers,
                ty,
                entries,
                ..
            } => {
                self.fmt_modifiers(modifiers);
                let parts: Vec<String> = entries
                    .iter()
                    .map(|e| self.property_entry_to_string(e, *ty))
                    .collect();
                self.write(&parts.join(", "));
                self.write(";");
            }
            ClassMember::PropertyHook {
                modifiers,
                ty,
                name,
                default,
                hooks,
                ..
            } => {
                self.fmt_modifiers(modifiers);
                if let Some(ty) = ty {
                    self.fmt_type(ty);
                    self.write(" ");
                }
                self.write(&self.token_text(name));
                if let Some(default) = default {
                    self.write(" = ");
                    self.fmt_expr(default);
                }
                for hook in *hooks {
                    self.write(" {");
                    self.fmt_property_hook_body(&hook.body);
                    self.write("}");
                }
                self.write(";");
            }
            ClassMember::Method {
                modifiers,
                name,
                params,
                return_type,
                body,
                ..
            } => {
                self.fmt_modifiers(modifiers);
                self.write("fn ");
                self.write(&self.token_text(name));
                self.write("(");
                self.fmt_param_list(params);
                self.write(")");
                if let Some(ty) = return_type {
                    self.write(" ");
                    self.fmt_type(ty);
                }
                self.write(" ");
                self.fmt_block(body);
            }
            ClassMember::Const {
                modifiers,
                ty,
                consts,
                ..
            } => {
                self.fmt_modifiers(modifiers);
                self.write("const ");
                if let Some(ty) = ty {
                    self.fmt_type(ty);
                    self.write(" ");
                }
                let parts: Vec<String> = consts
                    .iter()
                    .map(|c| {
                        let mut s = self.token_text(c.name).to_string();
                        s.push_str(" = ");
                        s.push_str(&self.expr_to_string(&c.value));
                        s
                    })
                    .collect();
                self.write(&parts.join(", "));
                self.write(";");
            }
            ClassMember::TraitUse { traits, .. } => {
                self.write("use ");
                self.write(
                    &traits
                        .iter()
                        .map(|t| self.name_text(t))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                self.write(";");
            }
            ClassMember::Embed { types, .. } => {
                // DekaScript struct embeddings are written as bare type names,
                // e.g. `struct Person { Label }`. This keeps the formatter
                // idempotent with the parser's bare-embed form.
                self.write(
                    &types
                        .iter()
                        .map(|t| self.name_text(t))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                self.write(";");
            }
            ClassMember::Case {
                name, value, payload, ..
            } => {
                self.write(&self.token_text(name));
                if let Some(payload) = payload {
                    self.write("(");
                    let types: Vec<String> = payload
                        .iter()
                        .filter_map(|p| p.ty.map(|t| self.type_to_string(t)))
                        .collect();
                    self.write(&types.join(", "));
                    self.write(")");
                }
                if let Some(value) = value {
                    self.write(" = ");
                    self.fmt_expr(value);
                }
                self.write(",");
            }
        }
    }

    fn fmt_modifiers(&mut self, modifiers: &[php_rs::parser::lexer::token::Token]) {
        if modifiers.is_empty() {
            return;
        }
        let text: Vec<String> = modifiers.iter().map(|m| self.token_text(m)).collect();
        self.write(&text.join(" "));
        self.write(" ");
    }

    fn property_entry_to_string(
        &self,
        entry: &PropertyEntry<'_>,
        ty: Option<&Type<'_>>,
    ) -> String {
        let mut s = String::new();
        if entry.is_mut {
            s.push_str("mut ");
        }
        s.push_str(&self.token_text(entry.name));
        if entry.optional {
            s.push('?');
        }
        if let Some(ty) = ty {
            s.push_str(": ");
            // If the entry already uses the `?` shorthand and the parser
            // represented the type as `Option<T>`, render the inner type so
            // we don't end up with `name?: T?`.
            if entry.optional && matches!(ty, Type::Option(_)) {
                if let Type::Option(inner) = ty {
                    s.push_str(&self.type_to_string(inner));
                } else {
                    s.push_str(&self.type_to_string(ty));
                }
            } else {
                s.push_str(&self.type_to_string(ty));
            }
        }
        for ann in entry.annotations {
            s.push_str(" @");
            s.push_str(&self.token_text(ann.name));
            if !ann.args.is_empty() {
                s.push('(');
                s.push_str(
                    &ann
                        .args
                        .iter()
                        .map(|a| self.expr_to_string(a))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                s.push(')');
            }
        }
        if let Some(default) = entry.default {
            s.push_str(" = ");
            s.push_str(&self.expr_to_string(default));
        }
        s
    }

    fn fmt_property_hook_body(&mut self, body: &PropertyHookBody<'_>) {
        match body {
            PropertyHookBody::None => {}
            PropertyHookBody::Statements(stmts) => {
                for stmt in *stmts {
                    self.fmt_stmt(stmt);
                }
            }
            PropertyHookBody::Expr(expr) => self.fmt_expr(expr),
        }
    }

    // --- function signatures -----------------------------------------------

    fn fmt_fn_sig(
        &mut self,
        is_async: bool,
        name: Option<&php_rs::parser::lexer::token::Token>,
        type_params: &[TypeParam<'_>],
        params: &[Param<'_>],
        return_type: Option<&Type<'_>>,
    ) {
        if is_async {
            self.write("async ");
        }
        self.write("fn");
        if let Some(name) = name {
            self.write(" ");
            self.write(&self.token_text(name));
        }
        if !type_params.is_empty() {
            self.write("<");
            self.fmt_type_param_list(type_params);
            self.write(">");
        }
        self.write("(");
        self.fmt_param_list(params);
        self.write(")");
        if let Some(ty) = return_type {
            self.write(" ");
            self.fmt_type(ty);
        }
    }

    fn fmt_receiver_method_sig(
        &mut self,
        is_async: bool,
        name: &php_rs::parser::lexer::token::Token,
        receiver: &Receiver<'_>,
        params: &[Param<'_>],
        return_type: Option<&Type<'_>>,
    ) {
        if is_async {
            self.write("async ");
        }
        self.write("fn (");
        self.write(&self.token_text(receiver.var));
        if receiver.is_mut {
            self.write(" mut");
        }
        self.write(" ");
        self.fmt_type(receiver.ty);
        self.write(") ");
        self.write(&self.token_text(name));
        self.write("(");
        self.fmt_param_list(params);
        self.write(")");
        if let Some(ty) = return_type {
            self.write(" ");
            self.fmt_type(ty);
        }
    }

    fn fmt_param_list(&mut self, params: &[Param<'_>]) {
        let parts: Vec<String> = params.iter().map(|p| self.param_to_string(p)).collect();
        self.write(&parts.join(", "));
    }

    fn param_to_string(&self, param: &Param<'_>) -> String {
        let mut s = String::new();
        for modifier in param.modifiers {
            s.push_str(&self.token_text(modifier));
            s.push(' ');
        }
        if param.by_ref {
            s.push_str("&");
        }
        if param.variadic {
            s.push_str("...");
        }
        s.push_str(&self.token_text(param.name));
        if let Some(ty) = param.ty {
            s.push_str(": ");
            s.push_str(&self.type_to_string(ty));
        }
        if let Some(default) = param.default {
            s.push_str(" = ");
            s.push_str(&self.expr_to_string(default));
        }
        s
    }

    fn fmt_type_param_list(&mut self, type_params: &[TypeParam<'_>]) {
        let parts: Vec<String> = type_params
            .iter()
            .map(|tp| {
                let mut s = self.token_text(tp.name).to_string();
                if let Some(constraint) = tp.constraint {
                    s.push_str(": ");
                    s.push_str(&self.type_to_string(constraint));
                }
                s
            })
            .collect();
        self.write(&parts.join(", "));
    }

    // --- types -------------------------------------------------------------

    fn fmt_type(&mut self, ty: &Type<'_>) {
        self.write(&self.type_to_string(ty));
    }

    fn type_to_string(&self, ty: &Type<'_>) -> String {
        match ty {
            Type::Simple(token) => self.token_text(token).to_string(),
            Type::Name(name) => self.name_text(name),
            Type::Union(parts) => parts
                .iter()
                .map(|t| self.type_to_string(t))
                .collect::<Vec<_>>()
                .join(" | "),
            Type::Intersection(parts) => parts
                .iter()
                .map(|t| self.type_to_string(t))
                .collect::<Vec<_>>()
                .join(" & "),
            Type::Option(inner) => format!("{}?", self.type_to_string(inner)),
            Type::ObjectShape(fields) => {
                let parts: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        let mut s = self.token_text(f.name).to_string();
                        if f.optional {
                            s.push('?');
                        }
                        s.push_str(": ");
                        s.push_str(&self.type_to_string(f.ty));
                        s
                    })
                    .collect();
                format!("{{ {} }}", parts.join("; "))
            }
            Type::Applied { base, args } => {
                let mut s = self.type_to_string(base);
                s.push('<');
                s.push_str(
                    &args
                        .iter()
                        .map(|a| self.type_to_string(a))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                s.push('>');
                s
            }
            Type::Function {
                params,
                return_type,
            } => {
                let mut s = "fn(".to_string();
                s.push_str(
                    &params
                        .iter()
                        .map(|p| self.type_to_string(p))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                s.push_str(") ");
                s.push_str(&self.type_to_string(return_type));
                s
            }
        }
    }

    // --- expressions -------------------------------------------------------

    fn fmt_expr(&mut self, expr: &Expr<'_>) {
        self.write(&self.expr_to_string(expr));
    }

    fn expr_to_string(&self, expr: &Expr<'_>) -> String {
        self.expr_to_string_with_prec(expr, Prec::Min)
    }

    fn expr_to_string_with_prec(&self, expr: &Expr<'_>, min_prec: Prec) -> String {
        let s = self.expr_inner_to_string(expr);
        if self.expr_prec(expr) < min_prec {
            format!("({})", s)
        } else {
            s
        }
    }

    fn expr_inner_to_string(&self, expr: &Expr<'_>) -> String {
        match expr {
            Expr::Assign { var, expr, .. } => {
                format!("{} = {}", self.expr_to_string(var), self.expr_to_string(expr))
            }
            Expr::AssignRef { var, expr, .. } => {
                format!("{} = &{}", self.expr_to_string(var), self.expr_to_string(expr))
            }
            Expr::AssignOp { var, op, expr, .. } => {
                let op_str = assign_op_str(op);
                format!(
                    "{} {} {}",
                    self.expr_to_string(var),
                    op_str,
                    self.expr_to_string(expr)
                )
            }
            Expr::Binary { left, op, right, .. } => {
                if matches!(op, BinaryOp::Pipe) {
                    return self.pipe_chain_to_string(expr);
                }
                let (op_prec, assoc, op_str) = binary_op_info(op);
                let (left_min, right_min) = match assoc {
                    Assoc::Left => (op_prec, op_prec.next()),
                    Assoc::Right => (op_prec.next(), op_prec),
                    Assoc::NonAssoc => (op_prec.next(), op_prec.next()),
                };
                format!(
                    "{} {} {}",
                    self.expr_to_string_with_prec(left, left_min),
                    op_str,
                    self.expr_to_string_with_prec(right, right_min)
                )
            }
            Expr::Unary { op, expr, .. } => {
                let (prefix, postfix) = unary_op_str(op);
                if !prefix.is_empty() {
                    format!("{}{}", prefix, self.expr_to_string_with_prec(expr, Prec::Unary))
                } else {
                    format!("{}{}", self.expr_to_string_with_prec(expr, Prec::Postfix), postfix)
                }
            }
            Expr::Call { func, args, .. } => {
                let mut s = self.expr_to_string(func);
                s.push('(');
                s.push_str(&self.arg_list_to_string(args));
                s.push(')');
                s
            }
            Expr::Array { items, .. } => {
                let mut s = "[".to_string();
                s.push_str(&self.array_item_list_to_string(items));
                s.push(']');
                s
            }
            Expr::ObjectLiteral { items, .. } => {
                let mut s = "{".to_string();
                if items.is_empty() {
                    s.push('}');
                } else {
                    let parts: Vec<String> = items
                        .iter()
                        .map(|item| {
                            let key = match &item.key {
                                ObjectKey::Ident(t) => self.token_text(t).to_string(),
                                ObjectKey::String(t) => self.token_text(t).to_string(),
                            };
                            format!("{}: {}", key, self.expr_to_string(item.value))
                        })
                        .collect();
                    s.push_str(&parts.join(", "));
                    s.push('}');
                }
                s
            }
            Expr::JsxElement {
                name,
                attributes,
                children,
                ..
            } => self.jsx_element_to_string(&self.name_text(name), attributes, children),
            Expr::JsxFragment { children, .. } => {
                self.jsx_element_to_string("", &[], children)
            }
            Expr::StructLiteral { name, fields, .. } => {
                let mut s = self.name_text(name);
                s.push_str(" { ");
                let parts: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        format!(
                            "{}: {}",
                            self.token_text(f.name),
                            self.expr_to_string(f.value)
                        )
                    })
                    .collect();
                s.push_str(&parts.join(", "));
                s.push_str(" }");
                s
            }
            Expr::ArrayDimFetch { array, dim: None, .. } => {
                format!("{}[]", self.expr_to_string(array))
            }
            Expr::ArrayDimFetch {
                array,
                dim: Some(dim),
                ..
            } => {
                format!("{}[{}]", self.expr_to_string(array), self.expr_to_string(dim))
            }
            Expr::DotAccess { target, property, .. } => {
                format!(
                    "{}.{}",
                    self.expr_to_string_with_prec(target, Prec::Postfix),
                    self.token_text(property)
                )
            }
            Expr::PropertyFetch { target, property, .. } => {
                format!(
                    "{}[{}]",
                    self.expr_to_string_with_prec(target, Prec::Postfix),
                    self.expr_to_string(property)
                )
            }
            Expr::MethodCall {
                target,
                method,
                args,
                ..
            } => {
                let mut s = self.expr_to_string_with_prec(target, Prec::Postfix);
                s.push('.');
                s.push_str(&self.expr_to_string(method));
                s.push('(');
                s.push_str(&self.arg_list_to_string(args));
                s.push(')');
                s
            }
            Expr::StaticCall {
                class,
                method,
                args,
                ..
            } => {
                let mut s = self.expr_to_string(class);
                s.push_str("::");
                s.push_str(&self.expr_to_string(method));
                s.push('(');
                s.push_str(&self.arg_list_to_string(args));
                s.push(')');
                s
            }
            Expr::ClassConstFetch { class, constant, .. } => {
                format!(
                    "{}::{}",
                    self.expr_to_string(class),
                    self.expr_to_string(constant)
                )
            }
            Expr::New { class, args, .. } => {
                let mut s = "new ".to_string();
                s.push_str(&self.expr_to_string(class));
                s.push('(');
                s.push_str(&self.arg_list_to_string(args));
                s.push(')');
                s
            }
            Expr::Variable { name, .. } => self.span_text(*name).to_string(),
            Expr::IndirectVariable { name, .. } => {
                format!("${{{}}}", self.expr_to_string(name))
            }
            Expr::Integer { value, .. } => std::str::from_utf8(value).unwrap_or("").to_string(),
            Expr::Float { value, .. } => std::str::from_utf8(value).unwrap_or("").to_string(),
            Expr::BigInt { value, .. } => std::str::from_utf8(value).unwrap_or("").to_string(),
            Expr::Boolean { value, .. } => {
                if *value {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            Expr::Null { .. } => "null".to_string(),
            Expr::String { value, .. } => std::str::from_utf8(value).unwrap_or("").to_string(),
            Expr::InterpolatedString { parts, .. } => {
                let mut s = "`".to_string();
                for part in *parts {
                    s.push_str(&self.interpolated_part_to_string(part));
                }
                s.push('`');
                s
            }
            Expr::ShellExec { parts, .. } => {
                let mut s = "`".to_string();
                for part in *parts {
                    s.push_str(&self.expr_to_string(part));
                }
                s.push('`');
                s
            }
            Expr::Include { kind, expr, .. } => {
                let kw = match kind {
                    IncludeKind::Include => "include",
                    IncludeKind::IncludeOnce => "include_once",
                    IncludeKind::Require => "require",
                    IncludeKind::RequireOnce => "require_once",
                };
                format!("{} {}", kw, self.expr_to_string(expr))
            }
            Expr::MagicConst { kind, .. } => magic_const_str(kind).to_string(),
            Expr::PostInc { var, .. } => {
                format!("{}++", self.expr_to_string_with_prec(var, Prec::Postfix))
            }
            Expr::PostDec { var, .. } => {
                format!("{}--", self.expr_to_string_with_prec(var, Prec::Postfix))
            }
            Expr::Ternary {
                condition,
                if_true,
                if_false,
                ..
            } => {
                let mut s = self.expr_to_string_with_prec(condition, Prec::Ternary);
                s.push_str(" ? ");
                if let Some(if_true) = if_true {
                    s.push_str(&self.expr_to_string(if_true));
                }
                s.push_str(" : ");
                s.push_str(&self.expr_to_string_with_prec(if_false, Prec::Ternary));
                s
            }
            Expr::Match { condition, arms, .. } => {
                let mut s = "match (".to_string();
                s.push_str(&self.expr_to_string(condition));
                s.push_str(") {");
                let arm_strs: Vec<String> = arms
                    .iter()
                    .map(|arm| self.match_arm_to_string(arm))
                    .collect();
                if arm_strs.is_empty() {
                    s.push('}');
                    return s;
                }

                let source_has_newline = arms.windows(2).any(|w| {
                    let prev_span = w[0].span;
                    let next_span = w[1].span;
                    if prev_span.end >= next_span.start {
                        return false;
                    }
                    self.source.as_bytes()[prev_span.end..next_span.start]
                        .iter()
                        .any(|&b| b == b'\n')
                });
                let single_line = arm_strs.join(", ");
                let use_multiline =
                    arms.len() >= 2 || source_has_newline || single_line.len() > 80;

                if !use_multiline {
                    s.push(' ');
                    s.push_str(&single_line);
                    s.push(' ');
                    s.push('}');
                } else {
                    let arm_indent = "  ".repeat(self.indent + 1);
                    for arm in arm_strs {
                        s.push('\n');
                        s.push_str(&arm_indent);
                        s.push_str(&arm);
                        s.push(',');
                    }
                    s.push('\n');
                    s.push_str(&"  ".repeat(self.indent));
                    s.push('}');
                }
                s
            }
            Expr::AnonymousClass {
                modifiers,
                args,
                extends,
                implements,
                members: _,
                ..
            } => {
                let mut s = String::new();
                for m in *modifiers {
                    s.push_str(&self.token_text(m));
                    s.push(' ');
                }
                s.push_str("new class");
                s.push('(');
                s.push_str(&self.arg_list_to_string(args));
                s.push(')');
                if let Some(base) = extends {
                    s.push_str(" : ");
                    s.push_str(&self.name_text(base));
                }
                if !implements.is_empty() {
                    s.push_str(" implements ");
                    s.push_str(
                        &implements
                            .iter()
                            .map(|n| self.name_text(n))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                s.push_str(" { /* anonymous class */ }");
                s
            }
            Expr::Print { expr, .. } => {
                format!("print({})", self.expr_to_string(expr))
            }
            Expr::Yield { key, value, from, .. } => {
                if *from {
                    format!(
                        "yield from {}",
                        value.map_or("".to_string(), |v| self.expr_to_string(v))
                    )
                } else {
                    let mut s = "yield".to_string();
                    if let Some(key) = key {
                        s.push(' ');
                        s.push_str(&self.expr_to_string(key));
                        s.push_str(" => ");
                    } else if let Some(_value) = value {
                        s.push(' ');
                    }
                    if let Some(value) = value {
                        s.push_str(&self.expr_to_string(value));
                    }
                    s
                }
            }
            Expr::Cast { kind, expr, .. } => {
                format!(
                    "({}) {}",
                    cast_kind_str(kind),
                    self.expr_to_string_with_prec(expr, Prec::Unary)
                )
            }
            Expr::Empty { expr, .. } => format!("empty({})", self.expr_to_string(expr)),
            Expr::Isset { vars, .. } => {
                let mut s = "isset(".to_string();
                s.push_str(&self.expr_list_to_string(vars, ", "));
                s.push(')');
                s
            }
            Expr::Eval { expr, .. } => format!("eval({})", self.expr_to_string(expr)),
            Expr::Await { expr, .. } => {
                format!("await {}", self.expr_to_string_with_prec(expr, Prec::Unary))
            }
            Expr::Die { expr: None, .. } => "die".to_string(),
            Expr::Die { expr: Some(expr), .. } => format!("die({})", self.expr_to_string(expr)),
            Expr::Exit { expr: None, .. } => "exit".to_string(),
            Expr::Exit { expr: Some(expr), .. } => format!("exit({})", self.expr_to_string(expr)),
            Expr::Closure {
                is_async,
                is_static,
                params,
                return_type,
                body,
                ..
            } => {
                let mut s = String::new();
                if *is_static {
                    s.push_str("static ");
                }
                if *is_async {
                    s.push_str("async ");
                }
                s.push_str("fn(");
                s.push_str(&self.param_list_to_string(params));
                s.push(')');
                if let Some(ty) = return_type {
                    s.push_str(" ");
                    s.push_str(&self.type_to_string(ty));
                }
                s.push_str(" { ");
                s.push_str(&self.stmt_list_to_string(body, " "));
                s.push_str(" }");
                s
            }
            Expr::ArrowFunction {
                is_async,
                is_static,
                params,
                return_type,
                expr,
                ..
            } => {
                let mut s = String::new();
                if *is_static {
                    s.push_str("static ");
                }
                if *is_async {
                    s.push_str("async ");
                }
                s.push_str("fn(");
                s.push_str(&self.param_list_to_string(params));
                s.push(')');
                if let Some(ty) = return_type {
                    s.push_str(" ");
                    s.push_str(&self.type_to_string(ty));
                }
                s.push_str(" => ");
                s.push_str(&self.expr_to_string(expr));
                s
            }
            Expr::Clone { expr, .. } => {
                format!("clone {}", self.expr_to_string_with_prec(expr, Prec::Unary))
            }
            Expr::NullsafePropertyFetch { target, property, .. } => {
                format!(
                    "{}?.{}",
                    self.expr_to_string_with_prec(target, Prec::Postfix),
                    self.expr_to_string(property)
                )
            }
            Expr::NullsafeMethodCall {
                target,
                method,
                args,
                ..
            } => {
                let mut s = self.expr_to_string_with_prec(target, Prec::Postfix);
                s.push_str("?.");
                s.push_str(&self.expr_to_string(method));
                s.push('(');
                s.push_str(&self.arg_list_to_string(args));
                s.push(')');
                s
            }
            Expr::VariadicPlaceholder { .. } => "...".to_string(),
            Expr::Spread { expr, .. } => {
                format!("...{}", self.expr_to_string(expr))
            }
            Expr::Bridge {
                kind, action, args, ..
            } => {
                let mut s = format!(
                    "bridge {}.{}(",
                    String::from_utf8_lossy(kind),
                    String::from_utf8_lossy(action)
                );
                s.push_str(&self.arg_list_to_string(args));
                s.push(')');
                s
            }
            Expr::Unsafe { raw, .. } => {
                // Preserve raw JavaScript inside `unsafe { ... }` verbatim. We
                // only normalize the surrounding whitespace, not the body.
                // If the user already supplied whitespace (or newlines) inside
                // the braces, do not inject extra spaces, otherwise each format
                // pass would grow the padding.
                let inner = String::from_utf8_lossy(raw);
                let has_surrounding_ws = inner.starts_with(' ') || inner.starts_with('\t') || inner.starts_with('\n')
                    || inner.ends_with(' ') || inner.ends_with('\t') || inner.ends_with('\n')
                    || inner.is_empty();
                if has_surrounding_ws {
                    format!("unsafe {{{inner}}}")
                } else {
                    format!("unsafe {{ {inner} }}")
                }
            }
            Expr::Cql { name, cypher, .. } => {
                let mut s = "cql ".to_string();
                s.push_str(&self.token_text(name));
                s.push_str(" = ");
                s.push_str(&self.span_text(*cypher));
                s
            }
            Expr::Error { .. } => "/* error */".to_string(),
        }
    }

    fn pipe_chain_to_string(&self, expr: &Expr<'_>) -> String {
        let mut chain: Vec<&Expr<'_>> = Vec::new();
        let mut current = expr;
        loop {
            match current {
                Expr::Binary {
                    op: BinaryOp::Pipe,
                    left,
                    right,
                    ..
                } => {
                    chain.push(*right);
                    current = *left;
                }
                _ => {
                    chain.push(current);
                    break;
                }
            }
        }
        chain.reverse();

        let source_has_newline = chain.windows(2).any(|w| {
            let prev = self.expr_span(w[0]);
            let next = self.expr_span(w[1]);
            if prev.end >= next.start {
                return false;
            }
            self.source.as_bytes()[prev.end..next.start]
                .iter()
                .any(|&b| b == b'\n')
        });

        let single_line = chain
            .iter()
            .map(|e| self.expr_to_string(e))
            .collect::<Vec<_>>()
            .join(" |> ");

        if !source_has_newline && single_line.len() <= 80 {
            return single_line;
        }

        let mut s = self.expr_to_string(chain[0]);
        let indent = "  ".repeat(self.indent + 1);
        for segment in &chain[1..] {
            s.push('\n');
            s.push_str(&indent);
            s.push_str("|> ");
            s.push_str(&self.expr_to_string(segment));
        }
        s
    }

    fn expr_prec(&self, expr: &Expr<'_>) -> Prec {
        match expr {
            Expr::Ternary { .. } => Prec::Ternary,
            Expr::Binary { op, .. } => binary_op_info(op).0,
            Expr::Unary { .. } => Prec::Unary,
            Expr::Assign { .. } | Expr::AssignRef { .. } | Expr::AssignOp { .. } => Prec::Min,
            _ => Prec::Max,
        }
    }

    fn fmt_expr_list(&mut self, exprs: &[ExprId<'_>], sep: &str) {
        self.write(&self.expr_list_to_string(exprs, sep));
    }

    fn expr_list_to_string(&self, exprs: &[ExprId<'_>], sep: &str) -> String {
        exprs
            .iter()
            .map(|e| self.expr_to_string(e))
            .collect::<Vec<_>>()
            .join(sep)
    }

    fn arg_list_to_string(&self, args: &[Arg<'_>]) -> String {
        args.iter()
            .map(|a| {
                let mut s = String::new();
                if a.unpack {
                    s.push_str("...");
                }
                if let Some(name) = a.name {
                    s.push_str(&self.token_text(name));
                    s.push_str(": ");
                }
                s.push_str(&self.expr_to_string(a.value));
                s
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn array_item_list_to_string(&self, items: &[ArrayItem<'_>]) -> String {
        items
            .iter()
            .map(|item| {
                let mut s = String::new();
                if item.unpack {
                    s.push_str("...");
                }
                if let Some(key) = item.key {
                    s.push_str(&self.expr_to_string(key));
                    s.push_str(": ");
                }
                s.push_str(&self.expr_to_string(item.value));
                s
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn param_list_to_string(&self, params: &[Param<'_>]) -> String {
        params.iter().map(|p| self.param_to_string(p)).collect::<Vec<_>>().join(", ")
    }

    fn stmt_list_to_string(&self, stmts: &[StmtId<'_>], sep: &str) -> String {
        stmts
            .iter()
            .map(|s| self.stmt_to_string(s))
            .collect::<Vec<_>>()
            .join(sep)
    }

    fn stmt_to_string(&self, stmt: &Stmt<'_>) -> String {
        // Simplified: format as a single-line statement for use in closure bodies.
        match stmt {
            Stmt::Return { expr: Some(expr), .. } => format!("return {}", self.expr_to_string(expr)),
            Stmt::Return { expr: None, .. } => "return".to_string(),
            Stmt::Expression { expr, .. } => self.expr_to_string(expr),
            _ => "/* stmt */".to_string(),
        }
    }

    fn interpolated_part_to_string(&self, expr: &Expr<'_>) -> String {
        match expr {
            Expr::String { value, .. } => {
                std::str::from_utf8(value).unwrap_or("").to_string()
            }
            _ => format!("${{{}}}", self.expr_to_string(expr)),
        }
    }

    fn match_arm_to_string(&self, arm: &MatchArm<'_>) -> String {
        let mut s = String::new();
        match &arm.conditions {
            Some(conditions) => {
                s.push_str(
                    &conditions
                        .iter()
                        .map(|c| self.expr_to_string(c))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            None => s.push_str("default"),
        }
        s.push_str(" => ");
        s.push_str(&self.expr_to_string(arm.body));
        s
    }

    fn jsx_element_to_string(
        &self,
        name: &str,
        attributes: &[JsxAttribute<'_>],
        children: &[JsxChild<'_>],
    ) -> String {
        let mut s = String::new();
        s.push('<');
        s.push_str(name);
        for attr in attributes {
            s.push(' ');
            let name_text = self.token_text(attr.name);
            // JSX spread attributes are parsed with the ellipsis as the name
            // and a Spread expression as the value. Render them as `{...expr}`.
            if name_text == "..." {
                if let Some(value) = attr.value {
                    s.push_str("{...");
                    if let Expr::Spread { expr, .. } = value {
                        s.push_str(&self.expr_to_string(expr));
                    } else {
                        s.push_str(&self.expr_to_string(value));
                    }
                    s.push('}');
                }
                continue;
            }
            s.push_str(&name_text);
            if let Some(value) = attr.value {
                s.push_str("={");
                s.push_str(&self.expr_to_string(value));
                s.push('}');
            }
        }
        if children.is_empty() {
            s.push_str(" />");
        } else {
            s.push('>');
            for child in children {
                match child {
                    JsxChild::Text(span) => {
                        let text = self.span_text(*span);
                        // Trim whitespace-only text nodes.
                        if !text.chars().all(|c| c.is_whitespace()) {
                            s.push_str(&text);
                        }
                    }
                    JsxChild::Expr(expr) => {
                        s.push_str("{");
                        s.push_str(&self.expr_to_string(expr));
                        s.push_str("}");
                    }
                }
            }
            s.push_str("</");
            s.push_str(name);
            s.push('>');
        }
        s
    }
}

// --- operator tables -------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prec {
    Min = 0,
    Ternary = 1,
    Or = 2,
    And = 3,
    Coalesce = 4,
    BitOr = 5,
    BitXor = 6,
    BitAnd = 7,
    Equality = 8,
    Relational = 9,
    Shift = 10,
    Add = 11,
    Mul = 12,
    Pow = 13,
    Unary = 14,
    Postfix = 15,
    Max = 16,
}

impl Prec {
    fn next(self) -> Self {
        match self {
            Prec::Min => Prec::Ternary,
            Prec::Ternary => Prec::Or,
            Prec::Or => Prec::And,
            Prec::And => Prec::Coalesce,
            Prec::Coalesce => Prec::BitOr,
            Prec::BitOr => Prec::BitXor,
            Prec::BitXor => Prec::BitAnd,
            Prec::BitAnd => Prec::Equality,
            Prec::Equality => Prec::Relational,
            Prec::Relational => Prec::Shift,
            Prec::Shift => Prec::Add,
            Prec::Add => Prec::Mul,
            Prec::Mul => Prec::Pow,
            Prec::Pow => Prec::Unary,
            Prec::Unary => Prec::Postfix,
            Prec::Postfix => Prec::Max,
            Prec::Max => Prec::Max,
        }
    }
}

#[derive(Clone, Copy)]
enum Assoc {
    Left,
    Right,
    NonAssoc,
}

fn binary_op_info(op: &BinaryOp) -> (Prec, Assoc, &'static str) {
    match op {
        BinaryOp::Or | BinaryOp::LogicalOr => (Prec::Or, Assoc::Left, "||"),
        BinaryOp::And | BinaryOp::LogicalAnd => (Prec::And, Assoc::Left, "&&"),
        BinaryOp::Coalesce => (Prec::Coalesce, Assoc::Left, "??"),
        BinaryOp::BitOr => (Prec::BitOr, Assoc::Left, "|"),
        BinaryOp::BitXor => (Prec::BitXor, Assoc::Left, "^"),
        BinaryOp::BitAnd => (Prec::BitAnd, Assoc::Left, "&"),
        BinaryOp::Eq | BinaryOp::EqEq | BinaryOp::EqEqEq => (Prec::Equality, Assoc::NonAssoc, "==="),
        BinaryOp::NotEq | BinaryOp::NotEqEq => (Prec::Equality, Assoc::NonAssoc, "!=="),
        BinaryOp::Lt => (Prec::Relational, Assoc::NonAssoc, "<"),
        BinaryOp::LtEq => (Prec::Relational, Assoc::NonAssoc, "<="),
        BinaryOp::Gt => (Prec::Relational, Assoc::NonAssoc, ">"),
        BinaryOp::GtEq => (Prec::Relational, Assoc::NonAssoc, ">="),
        BinaryOp::ShiftLeft => (Prec::Shift, Assoc::Left, "<<"),
        BinaryOp::ShiftRight => (Prec::Shift, Assoc::Left, ">>"),
        BinaryOp::Plus => (Prec::Add, Assoc::Left, "+"),
        BinaryOp::Minus => (Prec::Add, Assoc::Left, "-"),
        BinaryOp::Mul => (Prec::Mul, Assoc::Left, "*"),
        BinaryOp::Div => (Prec::Mul, Assoc::Left, "/"),
        BinaryOp::Mod => (Prec::Mul, Assoc::Left, "%"),
        BinaryOp::Pow => (Prec::Pow, Assoc::Right, "**"),
        BinaryOp::Concat => (Prec::Add, Assoc::Left, "."),
        BinaryOp::Pipe => (Prec::Min, Assoc::Left, "|>"),
        _ => (Prec::Min, Assoc::Left, "?"),
    }
}

fn assign_op_str(op: &AssignOp) -> &'static str {
    match op {
        AssignOp::Plus => "+=",
        AssignOp::Minus => "-=",
        AssignOp::Mul => "*=",
        AssignOp::Div => "/=",
        AssignOp::Mod => "%=",
        AssignOp::Concat => ".=",
        AssignOp::BitAnd => "&=",
        AssignOp::BitOr => "|=",
        AssignOp::BitXor => "^=",
        AssignOp::ShiftLeft => "<<=",
        AssignOp::ShiftRight => ">>=",
        AssignOp::Pow => "**=",
        AssignOp::Coalesce => "??=",
    }
}

fn unary_op_str(op: &UnaryOp) -> (&'static str, &'static str) {
    match op {
        UnaryOp::Plus => ("+", ""),
        UnaryOp::Minus => ("-", ""),
        UnaryOp::Not => ("!", ""),
        UnaryOp::BitNot => ("~", ""),
        UnaryOp::PreInc => ("++", ""),
        UnaryOp::PreDec => ("--", ""),
        UnaryOp::ErrorSuppress => ("@", ""),
        UnaryOp::Reference => ("&", ""),
    }
}

fn cast_kind_str(kind: &CastKind) -> &'static str {
    match kind {
        CastKind::Int => "int",
        CastKind::Bool => "bool",
        CastKind::Float => "float",
        CastKind::String => "string",
        CastKind::Array => "array",
        CastKind::Object => "object",
        CastKind::Unset => "unset",
        CastKind::Void => "void",
    }
}

fn magic_const_str(kind: &MagicConstKind) -> &'static str {
    match kind {
        MagicConstKind::Dir => "__DIR__",
        MagicConstKind::File => "__FILE__",
        MagicConstKind::Line => "__LINE__",
        MagicConstKind::Function => "__FUNCTION__",
        MagicConstKind::Class => "__CLASS__",
        MagicConstKind::Trait => "__TRAIT__",
        MagicConstKind::Method => "__METHOD__",
        MagicConstKind::Namespace => "__NAMESPACE__",
        MagicConstKind::Property => "__PROPERTY__",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ds(source: &str) {
        let arena = bumpalo::Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Ds);
        let program = parser.parse_program();
        assert!(program.errors.is_empty(), "parse errors: {:?}", program.errors);
    }

    #[test]
    fn normalizes_trailing_whitespace_and_eof() {
        let input = "fn add() int {\n  return 1;  \n}\n\n";
        let output = format_ds(input).unwrap();
        assert_eq!(output, "fn add() int {\n  return 1\n}\n");
    }

    #[test]
    fn adds_trailing_newline_when_missing() {
        let input = "const x = 1;";
        let output = format_ds(input).unwrap();
        assert_eq!(output, "const x = 1\n");
    }

    #[test]
    fn empty_source_stays_empty() {
        assert_eq!(format_ds("").unwrap(), "");
    }

    #[test]
    fn formats_function() {
        let input = "fn add(  a:int,b :  string   ) int{ return a+b; }";
        let output = format_ds(input).unwrap();
        assert!(output.contains("fn add(a: int, b: string) int {"), "got: {}", output);
        assert!(output.contains("  return a + b"), "got: {}", output);
    }

    #[test]
    fn formats_function_literal() {
        let input = "const f=fn(x:int) int{return x*x;};";
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("const f = fn(x: int) int { return x * x }"),
            "got: {}",
            output
        );
    }

    #[test]
    fn formats_struct() {
        let input = "struct Point{x:int;y:int}";
        let output = format_ds(input).unwrap();
        assert!(output.contains("struct Point {"), "got: {}", output);
        assert!(output.contains("  x: int;"), "got: {}", output);
        assert!(output.contains("  y: int;"), "got: {}", output);
    }

    #[test]
    fn formats_struct_literal() {
        let input = "fn origin() Point { return Point { x: 0, y: 0 }; }";
        let output = format_ds(input).unwrap();
        assert!(output.contains("Point { x: 0, y: 0 }"), "got: {}", output);
    }

    #[test]
    fn formats_receiver_method() {
        let input = "fn (p mut Person) setName(name: string) { p.name = name; }";
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("fn (p mut Person) setName(name: string) {"),
            "got: {}",
            output
        );
    }

    #[test]
    fn formats_match_expression() {
        let input = r#"
            enum Status { Loading, Ready, Failed }
            fn f(s: Status) int {
                return match (s) {
                    Status::Loading => 0,
                    Status::Ready => 1,
                    Status::Failed => 2,
                };
            }
        "#;
        let output = format_ds(input).unwrap();
        assert!(output.contains("match (s) {"), "got: {}", output);
        assert!(output.contains("Status::Loading => 0"), "got: {}", output);
    }

    #[test]
    fn formats_nested_match_patterns() {
        let input = r#"
            fn f(r: Result<Option<number>, string>) number {
                return match (r) {
                    Err(e) => 0,
                    Ok(None) => 1,
                    Ok(Some(v)) => v,
                }
            }
        "#;
        let output = format_ds(input).unwrap();
        assert!(output.contains("Ok(None) => 1"), "got: {}", output);
        assert!(output.contains("Ok(Some(v)) => v"), "got: {}", output);
    }

    #[test]
    fn formats_jsx_element() {
        let input = "fn View() Object { return <div class=\"test\">hello {name}</div>; }";
        let output = format_ds(input).unwrap();
        assert!(output.contains("<div class={\"test\"}>"), "got: {}", output);
        assert!(output.contains("hello {name}"), "got: {}", output);
        assert!(output.contains("</div>"), "got: {}", output);
    }

    #[test]
    fn formats_pipe_expression() {
        let input = "fn inc(x: int) int { return x + 1; }\nconst y = 5 |> inc;";
        let output = format_ds(input).unwrap();
        assert!(output.contains("5 |> inc"), "got: {}", output);
    }

    #[test]
    fn formats_type_alias() {
        let input = "type UserId=string;";
        let output = format_ds(input).unwrap();
        assert_eq!(output, "type UserId = string\n");
    }

    #[test]
    fn formats_let_binding() {
        let input = "fn f(){let x=1; x+=2;}";
        let output = format_ds(input).unwrap();
        assert!(output.contains("let x = 1"), "got: {}", output);
        assert!(output.contains("x += 2"), "got: {}", output);
    }

    #[test]
    fn leaves_invalid_source_unchanged() {
        let input = "fn f( { return 1;";
        let output = format_ds(input).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn formats_async_await() {
        let input = "async fn fetch() Promise<int> { return await 1; }";
        let output = format_ds(input).unwrap();
        assert!(output.contains("async fn fetch() Promise<int> {"), "got: {}", output);
        assert!(output.contains("return await 1"), "got: {}", output);
    }

    #[test]
    fn formats_interface() {
        let input = "interface Named { name: string; }";
        let output = format_ds(input).unwrap();
        assert!(output.contains("interface Named {"), "got: {}", output);
        assert!(output.contains("  name: string;"), "got: {}", output);
    }

    #[test]
    fn formats_enum() {
        let input = "enum Option<T> { Some(T), None }";
        let output = format_ds(input).unwrap();
        assert!(output.contains("enum Option<T> {"), "got: {}", output);
        assert!(output.contains("  Some(T),"), "got: {}", output);
        assert!(output.contains("  None,"), "got: {}", output);
    }

    #[test]
    fn preserves_blank_line_between_top_level_statements() {
        let input = "fn a() {}\n\nfn b() {}";
        let output = format_ds(input).unwrap();
        assert_eq!(output, "fn a() {}\n\nfn b() {}\n");
    }

    #[test]
    fn preserves_blank_line_inside_block() {
        let input = "fn f() {\n  let x = 1\n\n  let y = 2\n}";
        let output = format_ds(input).unwrap();
        assert_eq!(
            output,
            "fn f() {\n  let x = 1\n\n  let y = 2\n}\n"
        );
    }

    #[test]
    fn collapses_multiple_blank_lines_to_one() {
        let input = "fn a() {}\n\n\n\nfn b() {}";
        let output = format_ds(input).unwrap();
        assert_eq!(output, "fn a() {}\n\nfn b() {}\n");
    }

    #[test]
    fn keeps_pipe_chain_on_one_line_when_short() {
        let input = "const y = 5 |> inc";
        let output = format_ds(input).unwrap();
        assert_eq!(output, "const y = 5 |> inc\n");
    }

    #[test]
    fn keeps_pipe_chain_across_multiple_lines() {
        let input = "const x = 5\n  |> add(1)\n  |> console.log";
        let output = format_ds(input).unwrap();
        assert_eq!(
            output,
            "const x = 5\n  |> add(1)\n  |> console.log\n"
        );
    }

    #[test]
    fn breaks_long_pipe_chain_to_multiple_lines() {
        let input = "const result = initialValue |> transformWithLongName |> anotherVeryLongTransformationName |> finalTransform";
        let output = format_ds(input).unwrap();
        assert!(output.contains("\n  |> "), "expected multiline pipe, got: {}", output);
    }

    #[test]
    fn does_not_add_semicolons_to_statements() {
        let input = "const x = 1\nconst y = 2\nconsole.log(x + y)";
        let output = format_ds(input).unwrap();
        assert_eq!(
            output,
            "const x = 1\nconst y = 2\nconsole.log(x + y)\n"
        );
        parse_ds(&output);
    }

    #[test]
    fn struct_embed_does_not_spam_embed() {
        let input = r#"struct Person {}
struct Employee {
  Person
  name: string
}"#;
        let output = format_ds(input).unwrap();
        assert!(
            output.matches("embed").count() <= 1,
            "formatter spammed 'embed' tokens: {}",
            output
        );
        assert!(
            output.contains("struct Employee {"),
            "expected Employee struct to be preserved: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn import_span_does_not_leak_into_next_statement() {
        let input = "import { add } from \"./missing.ds\";\nconsole.log(add(1, 2))";
        let output = format_ds(input).unwrap();
        assert!(
            !output.contains("console\nconsole"),
            "formatter duplicated the next statement due to an overlapping import span: {}",
            output
        );
        assert!(
            output.contains("console.log(add(1, 2))"),
            "expected console.log statement to be preserved: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn export_named_span_does_not_leak_into_next_statement() {
        let input = "export { answer };\nconsole.log(answer)";
        let output = format_ds(input).unwrap();
        assert!(
            !output.contains("console\nconsole"),
            "formatter duplicated the next statement due to an overlapping export span: {}",
            output
        );
        assert!(
            output.contains("console.log(answer)"),
            "expected console.log statement to be preserved: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn formatted_output_re_parses_without_errors() {
        let input = r#"fn inc(x: int) int {
  return x + 1
}

const y = 5
  |> add(1)
  |> console.log

const a = 1
const b = 2
print(a + b)
"#;
        let output = format_ds(input).unwrap();
        parse_ds(&output);
    }

    #[test]
    fn closing_brace_on_own_line_for_struct() {
        let input = "struct User { name: string; email: string? }";
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("  email?: string;\n}\n"),
            "expected struct closing brace on own line, got: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn closing_brace_on_own_line_for_enum() {
        let input = "enum Status { Loading, Ready, Failed }";
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("  Failed,\n}\n"),
            "expected enum closing brace on own line, got: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn optional_field_does_not_double_question_mark() {
        let input = "struct User { name: string; email: string? }";
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("email?: string"),
            "expected canonical optional field, got: {}",
            output
        );
        assert!(
            !output.contains("email?: string?"),
            "double ? in optional field, got: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn match_expression_breaks_arms_to_multiple_lines() {
        let input = r#"const label = match (current) {
  Status.Loading => "Loading...",
  Status.Ready => "Ready",
  Status.Failed => "Failed",
  _ => "Unknown"
}"#;
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("  Status.Loading => \"Loading...\","),
            "expected multiline match arm, got: {}",
            output
        );
        assert!(
            output.contains("}\n"),
            "expected closing brace on own line, got: {}",
            output
        );
        parse_ds(&output);
        assert!(
            output.contains("_ =>"),
            "catch-all `_` must round-trip (deka#281), got: {}",
            output
        );
        assert!(
            !output.contains(" | "),
            "match arm conditions must use commas, not `|` (deka#281), got: {}",
            output
        );
    }

    #[test]
    fn match_preserves_default_catch_all() {
        let input = r#"fn label(n: number) string {
  return match (n) {
    1 => "one",
    default => "other",
  }
}"#;
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("default =>"),
            "default catch-all must round-trip (deka#281), got: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn match_multi_condition_arm_uses_commas() {
        let input = r#"enum A { One }
enum B { Two }
fn f(x: A|B) number {
  return match (x) {
    A::One, B::Two => 1,
  }
}"#;
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("A::One, B::Two =>"),
            "expected comma-separated match conditions (deka#281), got: {}",
            output
        );
        assert!(
            !output.contains("A::One | B::Two"),
            "must not rewrite comma arms to `|` (deka#281), got: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn jsx_spread_attribute_is_preserved() {
        let input = r#"const props = { name: "Deka" }
const el = <div {...props} />
console.log(el)"#;
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("<div {...props} />"),
            "expected JSX spread attribute to be preserved, got: {}",
            output
        );
        parse_ds(&output);
    }

    #[test]
    fn unsafe_block_does_not_grow_whitespace_on_reformat() {
        let input = r#"const r = unsafe { 1 + 2 }
console.log(r)"#;
        let once = format_ds(input).unwrap();
        let twice = format_ds(&once).unwrap();
        assert_eq!(
            once, twice,
            "formatter should be idempotent for unsafe blocks, got:\n{}",
            twice
        );
        assert!(
            once.contains("unsafe { 1 + 2 }"),
            "expected single spaces inside unsafe block, got: {}",
            once
        );
        parse_ds(&once);
    }

    #[test]
    fn unsafe_block_preserves_multiline_body() {
        let input = r#"const r = unsafe {
  const x = 1
  x + 2
}
console.log(r)"#;
        let output = format_ds(input).unwrap();
        assert!(
            output.contains("unsafe {\n  const x = 1\n  x + 2\n}"),
            "expected multiline unsafe body to be preserved, got: {}",
            output
        );
        let reformatted = format_ds(&output).unwrap();
        assert_eq!(
            output, reformatted,
            "formatter should be idempotent for multiline unsafe blocks"
        );
        parse_ds(&output);
    }
}
