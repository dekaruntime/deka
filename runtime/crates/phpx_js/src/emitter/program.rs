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
        // First pass: collect struct/enum names and struct embeds. Collecting
        // embeds here lets the emitter promote embedded methods after all
        // struct factories have been declared.
        for stmt in program.statements {
            match stmt {
                Stmt::Class {
                    kind: ClassKind::Struct,
                    name,
                    members,
                    ..
                } => {
                    let struct_name = self.token_name(name);
                    self.struct_names.insert(struct_name.clone());
                    let mut embeds = Vec::new();
                    for member in *members {
                        if let ClassMember::Embed { types, .. } = member {
                            for ty in *types {
                                embeds.push(self.name_last_segment(*ty));
                            }
                        }
                    }
                    if !embeds.is_empty() {
                        self.struct_embeds.insert(struct_name.clone(), embeds);
                    }
                    self.collect_ds_struct_field_metadata(&struct_name, *members);
                }
                Stmt::Enum { name, .. } => {
                    let enum_name = self.token_name(name);
                    self.enum_names.insert(enum_name);
                }
                _ => {}
            }
        }
        self.compute_empty_embeds();
        // Second pass: collect DekaScript receiver methods so they can be
        // emitted after every struct factory is declared, regardless of source
        // order.
        if self.meta.is_ds {
            self.collect_ds_receiver_methods(program)?;
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
                    ..
                } => {
                    // Struct names are collected in the first pass above.
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
                    | Stmt::TypeAlias { .. }
            );
            // DekaScript `const` initializers are evaluated at the point of
            // declaration and may reference preceding `let` bindings. Keeping
            // them in the runtime execution order preserves source semantics
            // and avoids TDZ-style errors (deka#184).
            let is_ds_runtime_const = self.meta.is_ds && matches!(stmt, Stmt::Const { .. });
            if is_decl && !is_ds_runtime_const {
                // Defense-in-depth: if the parser ever allows a declaration
                // inside a block, do not silently hoist it to module scope.
                if self.meta.is_ds && self.scopes.len() != 1 {
                    return Err(format!(
                        "internal emitter error: DekaScript declaration {:?} was encountered inside a block and would be hoisted",
                        std::mem::discriminant(*stmt)
                    ));
                }
                self.emit_stmt(*stmt)?;
            } else {
                self.emit_stmt_to_main(*stmt)?;
            }
        }
        if self.meta.is_ds {
            self.emit_ds_method_registrations()?;
        }
        self.emit_template_to_main()?;
        Ok(())
    }

    /// Emit the frontmatter template section (everything after the closing `---`)
    /// as a response body assignment. The template body is collected into a local
    /// string and then written to `globalThis.__phpxCurrentResponse.body`, which is
    /// what the browser tour and server-side handlers use as the rendered HTML.
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
        self.body.push_str(
            "globalThis.__phpxCurrentResponse = { status: 200, headers: {}, body: '' };\n",
        );
        self.body.push_str("let __phpxTemplateBody = '';\n");
        for line in template {
            // If the whole line is a JSX element, evaluate it instead of treating
            // it as literal text.
            if let Some(jsx_expr) = self.try_emit_template_jsx(line) {
                self.body.push_str(&format!(
                    "__phpxTemplateBody += String({} ?? '');\n",
                    jsx_expr
                ));
            } else {
                let mut last_end = 0;
                let chars: Vec<char> = line.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    if chars[i] == '{' {
                        let start = i;
                        i += 1;
                        // Allow whitespace between { and $.
                        while i < chars.len() && chars[i].is_ascii_whitespace() {
                            i += 1;
                        }
                        if i >= chars.len() || chars[i] != '$' {
                            i = start + 1;
                            continue;
                        }
                        i += 1;
                        // Allow whitespace between $ and the variable name.
                        while i < chars.len() && chars[i].is_ascii_whitespace() {
                            i += 1;
                        }
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
                        // Allow whitespace before the closing }.
                        while i < chars.len() && chars[i].is_ascii_whitespace() {
                            i += 1;
                        }
                        if expr_end > expr_start && chars.get(i) == Some(&'}') {
                            i += 1;
                            let literal = &line[last_end..start];
                            if !literal.is_empty() {
                                let escaped = serde_json::to_string(literal)
                                    .unwrap_or_else(|_| "\"\"".to_string());
                                self.body
                                    .push_str(&format!("__phpxTemplateBody += {};\n", escaped));
                            }
                            let expr: String = chars[expr_start..expr_end].iter().collect();
                            let js_expr = expr.replace('$', "");
                            if !js_expr.is_empty() {
                                self.body.push_str(&format!(
                                    "__phpxTemplateBody += String({} ?? '');\n",
                                    js_expr
                                ));
                            }
                            last_end = i;
                            continue;
                        }
                        i = start + 1;
                    }
                    i += 1;
                }
                let trailing = &line[last_end..];
                if !trailing.is_empty() || !line.is_empty() {
                    let escaped =
                        serde_json::to_string(trailing).unwrap_or_else(|_| "\"\"".to_string());
                    self.body
                        .push_str(&format!("__phpxTemplateBody += {};\n", escaped));
                }
            }
            // Preserve newlines between template lines.
            let newline = serde_json::to_string("\n").unwrap_or_else(|_| "\"\\n\"".to_string());
            self.body
                .push_str(&format!("__phpxTemplateBody += {};\n", newline));
        }
        self.body
            .push_str("globalThis.__phpxCurrentResponse.body = __phpxTemplateBody;\n");
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
                self.struct_names.insert(struct_name.clone());

                if self.meta.is_ds {
                    self.uses_deka_struct_helpers = true;
                    self.body.push_str(&format!(
                        "const {} = deka.Struct({});\n",
                        struct_name,
                        json_string(&struct_name)
                    ));
                    self.declare_in_scope(&struct_name);

                    // Struct-defined methods are immutable by default.
                    // They are registered after all struct factories are
                    // declared so that promoted/receiver methods are order-independent.
                    let struct_methods = self.emit_struct_methods(*members)?;
                    let ds_methods: Vec<DsMethod> = struct_methods
                        .into_iter()
                        .map(|(name, body)| DsMethod {
                            name,
                            body,
                            is_mut: false,
                        })
                        .collect();
                    if !ds_methods.is_empty() {
                        self.ds_struct_methods
                            .entry(struct_name.clone())
                            .or_default()
                            .extend(ds_methods);
                    }
                } else {
                    let methods = self.emit_struct_methods(*members)?;
                    if !methods.is_empty() {
                        self.struct_methods.insert(struct_name, methods);
                    }
                }
                Ok(())
            }
            Stmt::Enum { name, members, .. } => {
                self.emit_enum(name, members)?;
                Ok(())
            }
            Stmt::Class { .. } => {
                Err("class-like declarations are not supported in JS subset emitter".to_string())
            }
            Stmt::Trait { .. } => {
                // Traits are no longer part of DekaScript (RFD 19 Phase 5);
                // any trait declaration reaching the emitter is erased like an
                // interface. PHP legacy `trait` parsing is retained elsewhere.
                Ok(())
            }
            Stmt::Interface { .. } => {
                // Interfaces have no runtime representation in the JS subset;
                // they are erased like type annotations.
                Ok(())
            }
            Stmt::TypeAlias { .. } => {
                // Type aliases have no runtime representation; they are erased
                // like interfaces and other type-level declarations.
                Ok(())
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

                // Object-pattern parameters are lowered to a single synthetic
                // props argument followed by explicit `const` bindings. This
                // matches the component-call convention used by the JSX runtime
                // and keeps the emitted signature readable.
                let mut destructure_map: std::collections::HashMap<String, (String, Vec<String>)> =
                    std::collections::HashMap::new();
                let mut destructure_index = 0usize;
                for p in *params {
                    if let Some(php_rs::parser::ast::Type::ObjectShape(fields)) = p.ty {
                        let original = self.token_name(p.name);
                        let synthetic = if destructure_index == 0 {
                            "__phpx_props".to_string()
                        } else {
                            format!("__phpx_props_{}", destructure_index)
                        };
                        destructure_index += 1;
                        let field_names: Vec<String> =
                            fields.iter().map(|f| self.token_name(f.name)).collect();
                        destructure_map.insert(original, (synthetic, field_names));
                    }
                }

                let js_params = params
                    .iter()
                    .map(|p| {
                        let original = self.token_name(p.name);
                        let name = destructure_map
                            .get(&original)
                            .map(|(synthetic, _)| synthetic.clone())
                            .unwrap_or(original);
                        if p.variadic {
                            format!("...{}", name)
                        } else {
                            name
                        }
                    })
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
                    let original = self.token_name(p.name);
                    if let Some((synthetic, _)) = destructure_map.get(&original) {
                        self.declare_in_scope(synthetic);
                    } else {
                        self.declare_in_scope(&original);
                    }
                }

                // Emit `const field = props.field;` for each object-pattern field.
                for (_, (synthetic, fields)) in &destructure_map {
                    for field in fields {
                        self.body
                            .push_str(&format!("const {} = {}.{};\n", field, synthetic, field));
                        self.declare_in_scope(field);
                    }
                }

                self.emit_param_default_guards(params, &destructure_map)?;

                let destructure_sources: Vec<String> = destructure_map.keys().cloned().collect();
                for inner in *body {
                    if self.is_param_pattern_prologue(inner, &destructure_sources) {
                        continue;
                    }
                    self.emit_stmt(*inner)?;
                }
                self.function_scope_entry.pop();
                self.pop_scope();

                self.body.push_str("}\n\n");
                Ok(())
            }
            Stmt::ReceiverMethod {
                name,
                is_async,
                receiver,
                params,
                body,
                ..
            } => {
                // DekaScript receiver methods are collected in a pre-pass and
                // emitted after all struct factories are declared, so they are
                // order-independent.
                if self.meta.is_ds {
                    if self.scopes.len() != 1 {
                        return Err(
                            "receiver methods are only allowed at the top level in DekaScript"
                                .to_string(),
                        );
                    }
                    return Ok(());
                }
                let method_name = self.token_name(name);
                let receiver_name = self.token_name(receiver.var);
                let receiver_type_name = match receiver.ty {
                    AstType::Name(name) => self.name_last_segment(*name),
                    AstType::Simple(tok) => self.token_name(tok),
                    _ => {
                        return Err(
                            "receiver type must be a struct name in JS subset emitter".to_string(),
                        )
                    }
                };
                let js_params = std::iter::once(receiver_name)
                    .chain(params.iter().map(|p| self.token_name(p.name)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let block = self.emit_receiver_method_block(receiver, params, body)?;
                let async_kw = if *is_async { "async " } else { "" };
                self.uses_deka_struct_helpers = true;
                let func_expr = format!(
                    "{}function {}({}) {{\n{} }}",
                    async_kw, method_name, js_params, block
                );
                let reg_fn = if receiver.is_mut { "implMut" } else { "impl" };
                self.body.push_str(&format!(
                    "{}.{}({}, {});\n",
                    receiver_type_name,
                    reg_fn,
                    json_string(&method_name),
                    func_expr
                ));
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
                    if self.meta.is_ds {
                        self.uses_deka_freeze = true;
                        self.body
                            .push_str(&format!("const {} = deka.freeze({});\n", name, value));
                    } else {
                        self.body
                            .push_str(&format!("const {} = {};\n", name, value));
                    }
                    self.declare_immutable_in_scope(&name);
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
                let escaped =
                    serde_json::to_string(&text.to_string()).unwrap_or_else(|_| "\"\"".to_string());
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
        // Enums lower to frozen tagged plain objects, allocated once — never a
        // JS `class`. This matches how structs already lower
        // (`{"__struct": "Point", ...}`) and gives unit variants stable
        // identity (`Color.Red === Color.Red`) that survives JSON/
        // structuredClone round-trips, since `match` discriminates on the
        // `__enum`/`__case` tags rather than `instanceof` (which depends on a
        // prototype serialization discards). See deka#48, deka#49.
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

        // Method bodies (if any) are emitted once and attached as own
        // properties on every case object below — there is no shared
        // prototype to hang them on now that there is no class.
        let mut method_srcs: Vec<String> = Vec::new();
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
                method_srcs.push(format!(
                    "{}({}) {{\n{}    }}",
                    method_name, js_params, block
                ));
            }
        }

        // Fold in methods registered for this enum by PHPX struct/class
        // parsing (not DekaScript impl blocks, which were removed in RFD 19
        // Phase 5). Stored as full `function(...) { ... }` expressions so
        // they become `key: value` entries rather than shorthand-method
        // syntax; JS object literals permit freely mixing both forms, same as
        // `__enum`/`__case` above already do.
        if let Some(impl_methods) = self.struct_methods.get(&enum_name) {
            for (name, func_text) in impl_methods.clone() {
                method_srcs.push(format!("{}: {}", name, func_text));
            }
        }

        self.body
            .push_str(&format!("const {} = Object.freeze({{\n", enum_name));
        for (idx, case) in cases.iter().enumerate() {
            let mut entries = vec![
                format!("__enum: {}", json_string(&enum_name)),
                format!("__case: {}", json_string(&case.name)),
            ];
            entries.extend(
                case.params
                    .iter()
                    .map(|p| format!("{}: {}", json_string(p), p)),
            );
            entries.extend(method_srcs.iter().cloned());
            let object_body = entries.join(", ");
            let comma = if idx + 1 == cases.len() { "" } else { "," };
            if case.params.is_empty() {
                self.body.push_str(&format!(
                    "  {}: Object.freeze({{ {} }}){}\n",
                    case.name, object_body, comma
                ));
            } else {
                let param_list = case.params.join(", ");
                self.body.push_str(&format!(
                    "  {}: ({}) => Object.freeze({{ {} }}){}\n",
                    case.name, param_list, object_body, comma
                ));
            }
        }
        self.body.push_str("});\n");
        Ok(())
    }

    fn collect_ds_struct_field_metadata(
        &mut self,
        struct_name: &str,
        members: &[ClassMember<'_>],
    ) {
        let mut fields = Vec::new();
        for member in members {
            match member {
                ClassMember::Property { ty, entries, .. } => {
                    let optional_ty = ty.map(|t| self.type_is_optional(t)).unwrap_or(false);
                    for entry in *entries {
                        let name = self
                            .token_name(entry.name)
                            .trim_start_matches('$')
                            .to_string();
                        fields.push(DsStructField {
                            name,
                            optional: entry.optional || optional_ty,
                            empty_embed: false,
                        });
                    }
                }
                ClassMember::PropertyHook { ty, name, .. } => {
                    let optional_ty = ty.map(|t| self.type_is_optional(t)).unwrap_or(false);
                    let name = self.token_name(name).trim_start_matches('$').to_string();
                    fields.push(DsStructField {
                        name,
                        optional: optional_ty,
                        empty_embed: false,
                    });
                }
                ClassMember::Embed { types, .. } => {
                    for ty in *types {
                        let name = self.name_last_segment(*ty);
                        fields.push(DsStructField {
                            name,
                            optional: false,
                            empty_embed: false,
                        });
                    }
                }
                _ => {}
            }
        }
        self.struct_fields.insert(struct_name.to_string(), fields);
    }

    fn compute_empty_embeds(&mut self) {
        let names: Vec<String> = self.struct_fields.keys().cloned().collect();
        let mut emptiness: HashMap<String, bool> = HashMap::new();
        for name in &names {
            let empty = self.is_empty_embed_struct(name, &mut HashSet::new());
            emptiness.insert(name.clone(), empty);
        }
        for name in names {
            let embeds = self.struct_embeds.get(&name).cloned().unwrap_or_default();
            if let Some(fields) = self.struct_fields.get_mut(&name) {
                for field in fields.iter_mut() {
                    if embeds.contains(&field.name) {
                        field.empty_embed = *emptiness.get(&field.name).unwrap_or(&false);
                    }
                }
            }
        }
    }

    fn type_is_optional(&self, ty: &AstType<'_>) -> bool {
        match ty {
            AstType::Option(_) | AstType::Nullable(_) => true,
            AstType::Applied { base, .. } => {
                let name = match *base {
                    AstType::Simple(tok) => self.token_name(tok),
                    AstType::Name(name) => self.name_last_segment(*name),
                    _ => return false,
                };
                name.eq_ignore_ascii_case("Option")
            }
            _ => false,
        }
    }

    fn is_empty_embed_struct(&self, name: &str, visiting: &mut HashSet<String>) -> bool {
        if !visiting.insert(name.to_string()) {
            return true;
        }
        let Some(fields) = self.struct_fields.get(name) else {
            return false;
        };
        for field in fields {
            if self
                .struct_embeds
                .get(name)
                .map(|embeds| embeds.contains(&field.name))
                .unwrap_or(false)
            {
                if !self.is_empty_embed_struct(&field.name, visiting) {
                    return false;
                }
            } else {
                return false;
            }
        }
        true
    }
}
