use super::*;

impl<'a> JsSubsetEmitter<'a> {
    pub(crate) fn emit_program(&mut self, program: &Program<'_>) -> Result<(), String> {
        let import_locals: Vec<String> = self
            .meta
            .imports
            .iter()
            .flat_map(|decl| decl.specs.iter().map(|spec| spec.local.clone()))
            .collect();
        for local in import_locals {
            self.declare_in_scope(&local);
        }
        for stmt in program.statements {
            match stmt {
                Stmt::Function { name, .. } => {
                    let fn_name = self.token_name(name);
                    if !self.is_declared(&fn_name) {
                        self.declare_in_scope(&fn_name);
                    }
                }
                Stmt::Enum { name, .. } => {
                    let enum_name = self.token_name(name);
                    if !self.is_declared(&enum_name) {
                        self.declare_in_scope(&enum_name);
                    }
                }
                Stmt::Class {
                    kind: ClassKind::Struct,
                    name,
                    ..
                } => {
                    let struct_name = self.token_name(name);
                    self.struct_names.insert(struct_name);
                }
                _ => {}
            }
        }
        for stmt in program.statements {
            let is_decl = matches!(
                stmt,
                Stmt::Function { .. }
                    | Stmt::Enum { .. }
                    | Stmt::Class { .. }
                    | Stmt::Const { .. }
                    | Stmt::TypeAlias { .. }
            );
            if is_decl {
                self.emit_stmt(*stmt)?;
            } else {
                self.emit_stmt_to_main(*stmt)?;
            }
        }
        self.emit_template_to_main()?;
        Ok(())
    }

    /// Emit the frontmatter template section (everything after the closing `---`)
    /// as a series of `__dekaPrint` calls. This makes PHPX files with frontmatter
    /// render their markup body when executed, which is what the browser tour and
    /// server-side handlers expect.
    ///
    /// Simple `{$var}` / `{$obj.prop}` interpolations are translated to JS
    /// identifiers inline. Anything more complex is left as literal text.
    pub(super) fn emit_template_to_main(&mut self) -> Result<(), String> {
        let Some(template_start) = self.meta.template_start_line else {
            return Ok(());
        };
        let source_str = std::str::from_utf8(self.source).unwrap_or("");
        let lines: Vec<&str> = source_str.lines().collect();
        if template_start >= lines.len() {
            return Ok(());
        }
        let template: Vec<&str> = lines.iter().skip(template_start).copied().collect();
        if template.iter().all(|l| l.trim().is_empty()) {
            return Ok(());
        }

        std::mem::swap(&mut self.body, &mut self.main_body);
        for line in template {
            let mut last_end = 0;
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '{' && i + 1 < chars.len() && chars[i + 1] == '$' {
                    let start = i;
                    i += 2;
                    let expr_start = i;
                    while i < chars.len() {
                        let ch = chars[i];
                        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    let expr_end = i;
                    if expr_end > expr_start && chars.get(i) == Some(&'}') {
                        i += 1;
                        let literal = &line[last_end..start];
                        if !literal.is_empty() {
                            let escaped = serde_json::to_string(literal).unwrap_or_else(|_| "\"\"".to_string());
                            self.body.push_str(&format!(
                                "(globalThis.__dekaPrint ? globalThis.__dekaPrint({}) : console.log({}));\n",
                                escaped, escaped
                            ));
                        }
                        let expr: String = chars[expr_start..expr_end].iter().collect();
                        let js_expr = expr.replace('$', "");
                        if !js_expr.is_empty() {
                            self.body.push_str(&format!(
                                "(globalThis.__dekaPrint ? globalThis.__dekaPrint({}) : console.log({}));\n",
                                js_expr, js_expr
                            ));
                        }
                        last_end = i;
                        continue;
                    }
                    i = expr_start;
                }
                i += 1;
            }
            let trailing = &line[last_end..];
            if !trailing.is_empty() || !line.is_empty() {
                let escaped = serde_json::to_string(trailing).unwrap_or_else(|_| "\"\"".to_string());
                self.body.push_str(&format!(
                    "(globalThis.__dekaPrint ? globalThis.__dekaPrint({}) : console.log({}));\n",
                    escaped, escaped
                ));
            }
            // Preserve newlines between template lines.
            let newline = serde_json::to_string("\n").unwrap_or_else(|_| "\"\\n\"".to_string());
            self.body.push_str(&format!(
                "(globalThis.__dekaPrint ? globalThis.__dekaPrint({}) : console.log({}));\n",
                newline, newline
            ));
        }
        std::mem::swap(&mut self.body, &mut self.main_body);
        Ok(())
    }

    pub(super) fn emit_stmt_to_main(&mut self, stmt: StmtId<'_>) -> Result<(), String> {
        std::mem::swap(&mut self.body, &mut self.main_body);
        let res = self.emit_stmt(stmt);
        std::mem::swap(&mut self.body, &mut self.main_body);
        res
    }

    pub(super) fn emit_stmt(&mut self, stmt: StmtId<'_>) -> Result<(), String> {
        match stmt {
            Stmt::Namespace { .. } => {
                Err("namespace declarations are not supported in JS subset emitter".to_string())
            }
            Stmt::Use { .. } => {
                Err("use declarations are not supported in JS subset emitter".to_string())
            }
            Stmt::Class {
                kind: ClassKind::Struct,
                name,
                members,
                ..
            } => {
                let schema = self.emit_struct_schema(*members);
                let struct_name = self.token_name(name);
                self.struct_schemas.push((struct_name.clone(), schema));
                let methods = self.emit_struct_methods(*members)?;
                if !methods.is_empty() {
                    self.struct_methods.insert(struct_name, methods);
                }
                Ok(())
            }
            Stmt::Enum { name, members, .. } => {
                self.emit_enum(name, members)?;
                Ok(())
            }
            Stmt::Class { .. } | Stmt::Trait { .. } | Stmt::Interface { .. } => {
                Err("class-like declarations are not supported in JS subset emitter".to_string())
            }
            Stmt::TypeAlias { .. } => {
                Err("type aliases are not supported in JS subset emitter".to_string())
            }
            Stmt::Error { .. } => {
                Err("parser error statement reached JS subset emitter".to_string())
            }
            Stmt::Function {
                name,
                params,
                body,
                is_async,
                ..
            } => {
                let fn_name = self.token_name(name);
                if !self.is_declared(&fn_name) {
                    self.declare_in_scope(&fn_name);
                }
                let js_params = params
                    .iter()
                    .map(|p| self.token_name(p.name))
                    .collect::<Vec<_>>()
                    .join(", ");

                let exported =
                    self.scopes.len() == 1 && self.meta.exported_functions.contains(&fn_name);
                let async_kw = if *is_async { "async " } else { "" };
                if exported {
                    self.body.push_str(&format!(
                        "export {}function {}({}) {{\n",
                        async_kw, fn_name, js_params
                    ));
                } else {
                    self.body.push_str(&format!(
                        "{}function {}({}) {{\n",
                        async_kw, fn_name, js_params
                    ));
                }

                self.push_scope();
                self.function_scope_entry.push(self.scopes.len() - 1);
                for p in *params {
                    self.declare_in_scope(&self.token_name(p.name));
                }
                self.emit_param_default_guards(params)?;
                for inner in *body {
                    self.emit_stmt(*inner)?;
                }
                self.function_scope_entry.pop();
                self.pop_scope();

                self.body.push_str("}\n\n");
                Ok(())
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                let cond = self.emit_expr(*condition)?;
                self.body.push_str(&format!("if ({}) {{\n", cond));
                self.push_scope();
                for inner in *then_block {
                    self.emit_stmt(*inner)?;
                }
                self.pop_scope();
                self.body.push('}');

                if let Some(else_stmts) = else_block {
                    self.body.push_str(" else {\n");
                    self.push_scope();
                    for inner in *else_stmts {
                        self.emit_stmt(*inner)?;
                    }
                    self.pop_scope();
                    self.body.push('}');
                }
                self.body.push('\n');
                Ok(())
            }
            Stmt::Block { statements, .. } => {
                self.body.push_str("{\n");
                self.push_scope();
                for inner in *statements {
                    self.emit_stmt(*inner)?;
                }
                self.pop_scope();
                self.body.push_str("}\n");
                Ok(())
            }
            Stmt::While {
                condition, body, ..
            } => {
                let cond = self.emit_expr(*condition)?;
                self.body.push_str(&format!("while ({}) {{\n", cond));
                self.push_scope();
                for inner in *body {
                    self.emit_stmt(*inner)?;
                }
                self.pop_scope();
                self.body.push_str("}\n");
                Ok(())
            }
            Stmt::For {
                init,
                condition,
                loop_expr,
                body,
                ..
            } => {
                self.push_scope();
                let init_js = self.emit_for_init(init)?;
                let cond_js = self.emit_expr_list(condition)?;
                let loop_js = self.emit_expr_list(loop_expr)?;
                self.body
                    .push_str(&format!("for ({}; {}; {}) {{\n", init_js, cond_js, loop_js));
                for inner in *body {
                    self.emit_stmt(*inner)?;
                }
                self.body.push_str("}\n");
                self.pop_scope();
                Ok(())
            }
            Stmt::Foreach {
                expr,
                key_var,
                value_var,
                body,
                ..
            } => {
                let iterable = self.emit_expr(*expr)?;
                let value_name = self.extract_var_name(*value_var)?;
                if let Some(key_var) = key_var {
                    let key_name = self.extract_var_name(*key_var)?;
                    self.body.push_str(&format!(
                        "for (const [{} , {}] of Object.entries({})) {{\n",
                        key_name, value_name, iterable
                    ));
                    self.push_scope();
                    self.declare_in_scope(&key_name);
                    self.declare_in_scope(&value_name);
                } else {
                    self.body.push_str(&format!(
                        "for (const {} of (Array.isArray({}) ? {} : Object.values(({} ?? {{}})))) {{\n",
                        value_name, iterable, iterable, iterable
                    ));
                    self.push_scope();
                    self.declare_in_scope(&value_name);
                }
                for inner in *body {
                    self.emit_stmt(*inner)?;
                }
                self.pop_scope();
                self.body.push_str("}\n");
                Ok(())
            }
            Stmt::DoWhile {
                body, condition, ..
            } => {
                self.body.push_str("do {\n");
                self.push_scope();
                for inner in *body {
                    self.emit_stmt(*inner)?;
                }
                self.pop_scope();
                let cond = self.emit_expr(*condition)?;
                self.body.push_str(&format!("}} while ({});\n", cond));
                Ok(())
            }
            Stmt::Switch {
                condition, cases, ..
            } => {
                let cond = self.emit_expr(*condition)?;
                self.body.push_str(&format!("switch ({}) {{\n", cond));
                for case in *cases {
                    if let Some(case_cond) = case.condition {
                        let case_expr = self.emit_expr(case_cond)?;
                        self.body.push_str(&format!("case {}:\n", case_expr));
                    } else {
                        self.body.push_str("default:\n");
                    }
                    self.push_scope();
                    for inner in case.body {
                        self.emit_stmt(*inner)?;
                    }
                    self.pop_scope();
                }
                self.body.push_str("}\n");
                Ok(())
            }
            Stmt::Const { consts, .. } => {
                for item in *consts {
                    let name = self.token_name(item.name);
                    let value = self.emit_expr(item.value)?;
                    self.body
                        .push_str(&format!("const {} = {};\n", name, value));
                    self.declare_in_scope(&name);
                }
                Ok(())
            }
            Stmt::Global { vars, .. } => {
                for var in *vars {
                    if let Expr::Variable { name, .. } = *var {
                        let local = self.span_name(*name);
                        if local.is_empty() {
                            continue;
                        }
                        if !self.is_declared(&local) {
                            self.declare_in_scope(&local);
                            self.body
                                .push_str(&format!("let {} = globalThis.{};\n", local, local));
                        } else {
                            self.body
                                .push_str(&format!("{} = globalThis.{};\n", local, local));
                        }
                    }
                }
                Ok(())
            }
            Stmt::Static { vars, .. } => {
                for item in *vars {
                    let mut init_expr = item.default;
                    let target_expr = match *item.var {
                        Expr::Assign { var, expr, .. } => {
                            if init_expr.is_none() {
                                init_expr = Some(expr);
                            }
                            var
                        }
                        _ => item.var,
                    };

                    if let Some(raw_name) = self.extract_static_var_name(target_expr) {
                        let var_name = self
                            .sanitize_name(raw_name.split('=').next().unwrap_or(raw_name.as_str()));
                        let init = if let Some(default) = init_expr {
                            self.emit_expr(default)?
                        } else {
                            "undefined".to_string()
                        };
                        if !self.is_declared(&var_name) {
                            self.body
                                .push_str(&format!("let {} = {};\n", var_name, init));
                            self.declare_in_scope(&var_name);
                        } else {
                            self.body.push_str(&format!("{} = {};\n", var_name, init));
                        }
                    } else {
                        self.body.push_str(
                            "// unsupported static declaration target in JS subset mode\n",
                        );
                    }
                }
                Ok(())
            }
            Stmt::Unset { vars, .. } => {
                for var in *vars {
                    let target = self.emit_expr(*var)?;
                    self.body
                        .push_str(&format!("{} = {};\n", target, "undefined"));
                }
                Ok(())
            }
            Stmt::Label { name, .. } => {
                self.body.push_str(&format!(
                    "// label {} ignored in JS subset mode\n",
                    self.token_name(name)
                ));
                Ok(())
            }
            Stmt::Goto { label, .. } => {
                self.body.push_str(&format!(
                    "// goto {} is not supported in JS subset mode\n",
                    self.token_name(label)
                ));
                Ok(())
            }
            Stmt::Declare { body, .. } => {
                // PHP declare directives have no JS equivalent; emit body directly.
                self.push_scope();
                for inner in *body {
                    self.emit_stmt(*inner)?;
                }
                self.pop_scope();
                Ok(())
            }
            Stmt::HaltCompiler { .. } => {
                self.body
                    .push_str("// __halt_compiler ignored in JS subset mode\n");
                Ok(())
            }
            Stmt::Break { .. } => {
                self.body.push_str("break;\n");
                Ok(())
            }
            Stmt::Continue { .. } => {
                self.body.push_str("continue;\n");
                Ok(())
            }
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    let value = self.emit_expr(*expr)?;
                    self.body.push_str(&format!("return {};\n", value));
                } else {
                    self.body.push_str("return;\n");
                }
                Ok(())
            }
            Stmt::Throw { expr, .. } => {
                let value = self.emit_expr(*expr)?;
                self.body.push_str(&format!("throw {};\n", value));
                Ok(())
            }
            Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
                self.body.push_str("try {\n");
                self.push_scope();
                for inner in *body {
                    self.emit_stmt(*inner)?;
                }
                self.pop_scope();
                self.body.push_str("}");

                if let Some(first_catch) = catches.first() {
                    let err_name = if let Some(var) = first_catch.var {
                        let name = self.token_name(var);
                        if name.is_empty() {
                            "err".to_string()
                        } else {
                            name
                        }
                    } else {
                        "err".to_string()
                    };
                    self.body.push_str(&format!(" catch ({}) {{\n", err_name));
                    self.push_scope();
                    self.declare_in_scope(&err_name);
                    for inner in first_catch.body {
                        self.emit_stmt(*inner)?;
                    }
                    self.pop_scope();
                    self.body.push_str("}");
                }

                if let Some(finally_block) = finally {
                    self.body.push_str(" finally {\n");
                    self.push_scope();
                    for inner in *finally_block {
                        self.emit_stmt(*inner)?;
                    }
                    self.pop_scope();
                    self.body.push_str("}");
                }

                self.body.push('\n');
                Ok(())
            }
            Stmt::Echo { exprs, .. } => {
                for expr in *exprs {
                    let value = self.emit_expr(*expr)?;
                    self.body.push_str(&format!(
                        "(globalThis.__dekaPrint ? globalThis.__dekaPrint({}) : console.log({}));\n",
                        value, value
                    ));
                }
                Ok(())
            }
            Stmt::Expression { expr, .. } => {
                let assigned_kind = match *expr {
                    Expr::Assign { expr: rhs, .. } | Expr::AssignRef { expr: rhs, .. } => {
                        self.infer_expr_kind(rhs)
                    }
                    _ => None,
                };
                if let Some((name, rhs)) = self.assignment_to_named_var(*expr)? {
                    let top_level = self.scopes.len() == 1;
                    // Use function-local scope check when inside a function body:
                    // PHP variables are always function-scoped, so `$x = v` inside
                    // a function always creates a NEW local `x`, even if there is a
                    // module-level binding with the same name (e.g. an import).
                    let already_declared = if !self.function_scope_entry.is_empty() {
                        self.is_declared_in_current_function(&name)
                    } else {
                        self.is_declared(&name)
                    };
                    if !already_declared {
                        self.declare_in_scope(&name);
                        self.record_value_kind(&name, assigned_kind);
                        self.body.push_str(&format!("let {} = {};\n", name, rhs));
                        if top_level {
                            self.body
                                .push_str(&format!("globalThis.{} = {};\n", name, name));
                        }
                    } else {
                        self.record_value_kind(&name, assigned_kind);
                        self.body.push_str(&format!("{} = {};\n", name, rhs));
                        if top_level {
                            self.body
                                .push_str(&format!("globalThis.{} = {};\n", name, name));
                        }
                    }
                } else {
                    let value = self.emit_expr(*expr)?;
                    self.body.push_str(&format!("{};\n", value));
                }
                Ok(())
            }
            Stmt::InlineHtml { value, .. } => {
                let text = String::from_utf8_lossy(value);
                let escaped = serde_json::to_string(&text.to_string())
                    .unwrap_or_else(|_| "\"\"".to_string());
                self.body.push_str(&format!(
                    "(globalThis.__dekaPrint ? globalThis.__dekaPrint({}) : console.log({}));\n",
                    escaped, escaped
                ));
                Ok(())
            }
            Stmt::Nop { .. } => Ok(()),
        }
    }

    pub(super) fn emit_enum(
        &mut self,
        name: &php_rs::parser::lexer::token::Token,
        members: &[ClassMember<'_>],
    ) -> Result<(), String> {
        let enum_name = self.token_name(name);
        if !self.is_declared(&enum_name) {
            self.declare_in_scope(&enum_name);
        }
        let mut cases: Vec<EnumCaseDef> = Vec::new();
        let mut methods: Vec<&ClassMember<'_>> = Vec::new();

        for member in members {
            match member {
                ClassMember::Case { name, payload, .. } => {
                    let case_name = self.token_name(name);
                    let params = payload
                        .map(|items| {
                            items
                                .iter()
                                .map(|p| self.token_name(p.name))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    cases.push(EnumCaseDef {
                        name: case_name,
                        params,
                    });
                }
                ClassMember::Method { .. } => methods.push(member),
                _ => {
                    return Err("enum members other than cases/methods are not supported in JS subset emitter".to_string());
                }
            }
        }

        self.enum_cases.insert(enum_name.clone(), cases.clone());

        self.body.push_str(&format!("class {} {{\n", enum_name));
        self.body.push_str("  constructor(__case, __payload) {\n");
        self.body
            .push_str(&format!("    this.__enum = {};\n", json_string(&enum_name)));
        self.body.push_str("    this.__case = __case;\n");
        self.body.push_str("    if (__payload) {\n");
        self.body
            .push_str("      Object.assign(this, __payload);\n");
        self.body.push_str("    }\n");
        self.body.push_str("  }\n");

        for member in methods {
            if let ClassMember::Method {
                name, params, body, ..
            } = member
            {
                let method_name = self.token_name(name);
                let js_params = params
                    .iter()
                    .map(|p| self.token_name(p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let block = self.emit_method_block(params, body)?;
                self.body.push_str(&format!(
                    "  {}({}) {{\n{}  }}\n",
                    method_name, js_params, block
                ));
            }
        }

        for case in &cases {
            if case.params.is_empty() {
                self.body.push_str(&format!(
                    "  static get {}() {{ return new {}({}, null); }}\n",
                    case.name,
                    enum_name,
                    json_string(&case.name)
                ));
            } else {
                let param_list = case.params.join(", ");
                let payload_entries = case
                    .params
                    .iter()
                    .map(|p| format!("{}: {}", json_string(p), p))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.body.push_str(&format!(
                    "  static {}({}) {{ return new {}({}, {{{}}}); }}\n",
                    case.name,
                    param_list,
                    enum_name,
                    json_string(&case.name),
                    payload_entries
                ));
            }
        }

        self.body.push_str("}\n");
        Ok(())
    }
}
