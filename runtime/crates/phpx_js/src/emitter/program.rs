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
        // First pass: collect struct and enum names so impl-block routing
        // can decide whether a DekaScript impl targets a struct (factory)
        // or an enum (legacy tagged-object registry).
        for stmt in program.statements {
            match stmt {
                Stmt::Class {
                    kind: ClassKind::Struct,
                    name,
                    ..
                } => {
                    let struct_name = self.token_name(name);
                    self.struct_names.insert(struct_name);
                }
                Stmt::Enum { name, .. } => {
                    let enum_name = self.token_name(name);
                    self.enum_names.insert(enum_name);
                }
                _ => {}
            }
        }
        // RFD 19: collect each trait's default method bodies (non-empty
        // bodies only -- an abstract signature has nothing to fall back to
        // and must always be overridden, conformance-checked separately)
        // BEFORE the main pre-pass below processes any `impl` block. This
        // has to be a fully separate, earlier pass, not folded into the
        // loop underneath: a trait declared AFTER its impl in source order
        // must still be visible when that impl needs its defaults, the
        // exact same order-independence reasoning as the enum-impl fix
        // (deka#71) directly below.
        for stmt in program.statements {
            if let Stmt::Trait { name, members, .. } = stmt {
                let trait_name = self.token_name(name);
                // Only members with a non-empty body are real defaults --
                // an abstract signature (empty body) has nothing to fall
                // back to and must always be overridden by the impl
                // (conformance-checked separately in the typechecker).
                // Filter to a fresh owned Vec first rather than zipping
                // emit_struct_methods' output against `members` positionally
                // -- ClassMember has non-Method variants too (Property,
                // PropertyHook, ...), so a zip would silently misalign the
                // moment a trait ever mixes member kinds.
                let default_members: Vec<ClassMember<'_>> = members
                    .iter()
                    .copied()
                    .filter(|m| matches!(m, ClassMember::Method { body, .. } if !body.is_empty()))
                    .collect();
                if !default_members.is_empty() {
                    let defaults = self.emit_struct_methods(&default_members)?;
                    self.trait_default_methods
                        .entry(trait_name)
                        .or_default()
                        .extend(defaults);
                }
            }
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
                Stmt::Impl {
                    trait_name,
                    target,
                    members,
                    is_mut,
                    ..
                } => {
                    // RFD 19 (deka#71): registering impl-provided methods
                    // here, in the pre-pass, rather than when Stmt::Impl is
                    // reached in the main emission loop below, is what
                    // makes this order-independent -- an `impl` appearing
                    // AFTER the `enum`/`struct` it targets must still be
                    // visible when that declaration emits itself.
                    let target_name = self.token_name(&target.parts[0]);
                    let methods = self.emit_struct_methods(*members)?;
                    let mut tagged_methods: Vec<(String, String, bool)> = methods
                        .into_iter()
                        .map(|(name, body)| (name, body, *is_mut))
                        .collect();
                    // RFD 19: a trait impl that doesn't override one of the
                    // trait's default methods still needs that default's
                    // body to actually be callable -- the typechecker
                    // already allows this (a non-overridden default
                    // satisfies conformance, see check_trait_conflicts),
                    // but codegen was only ever emitting what the impl
                    // block's own members provided, so `x.defaultMethod()`
                    // threw "is not a function" at runtime even though
                    // `deka check` passed clean. Merge in any trait default
                    // whose name isn't already provided by this impl.
                    if let Some(trait_name) = trait_name {
                        let trait_key = self.token_name(&trait_name.parts[0]);
                        if let Some(defaults) = self.trait_default_methods.get(&trait_key).cloned()
                        {
                            let provided: std::collections::HashSet<String> = tagged_methods
                                .iter()
                                .map(|(name, _, _)| name.clone())
                                .collect();
                            for (name, body) in defaults {
                                if !provided.contains(&name) {
                                    tagged_methods.push((name, body, false));
                                }
                            }
                        }
                    }
                    if tagged_methods.is_empty() {
                        continue;
                    }
                    if self.meta.is_ds && !self.enum_names.contains(&target_name) {
                        // DekaScript structs register methods on the factory
                        // via Point.impl({...}) / Point.implMut({...}). Collect
                        // them here so the struct factory emission can merge
                        // struct-defined and impl-defined methods
                        // order-independently. Impls targeting enums stay on
                        // the legacy registry.
                        self.ds_impl_methods
                            .entry(target_name)
                            .or_default()
                            .extend(tagged_methods);
                    } else {
                        // PHPX/enum path keeps the legacy globalThis registry.
                        self.struct_methods
                            .entry(target_name)
                            .or_default()
                            .extend(tagged_methods.into_iter().map(|(n, b, _)| (n, b)));
                    }
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
                    let struct_methods = self.emit_struct_methods(*members)?;
                    let mut methods: Vec<(String, String, bool)> = struct_methods
                        .into_iter()
                        .map(|(name, body)| (name, body, false))
                        .collect();
                    if let Some(impl_methods) = self.ds_impl_methods.remove(&struct_name) {
                        let mut seen = std::collections::HashSet::new();
                        for (name, body, _) in methods.iter().cloned() {
                            seen.insert(name.clone());
                            let _ = body;
                        }
                        for (name, body, is_mut) in impl_methods {
                            if seen.insert(name.clone()) {
                                methods.push((name, body, is_mut));
                            }
                        }
                    }
                    self.emit_ds_impl_calls(&struct_name, &methods)?;
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
                // DekaScript traits (RFD 19) are a compile-time contract only
                // -- any Stmt::Trait reaching the emitter is the .ds meaning,
                // since the legacy PHP meaning is already rejected earlier by
                // validate_no_oop. Erased like an interface; `impl` blocks
                // are what produce runtime methods.
                Ok(())
            }
            Stmt::Impl {
                trait_name: _,
                target,
                members: _,
                ..
            } => {
                // PHPX/enum path: registration into self.struct_methods
                // already happened in the pre-pass. This arm is pure erasure.
                //
                // DekaScript path: local struct factories already consumed
                // their impl methods at the declaration site. If the target
                // is not declared in this module (e.g. an imported struct),
                // emit the impl call here.
                if self.meta.is_ds {
                    let target_name = self.token_name(&target.parts[0]);
                    if !self.struct_names.contains(&target_name) {
                        if let Some(methods) = self.ds_impl_methods.remove(&target_name) {
                            self.uses_deka_struct_helpers = true;
                            self.emit_ds_impl_calls(&target_name, &methods)?;
                        }
                    }
                }
                Ok(())
            }
            Stmt::Interface { .. } => {
                // Interfaces have no runtime representation in the JS subset;
                // they are erased like type annotations.
                Ok(())
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

        // RFD 19 (deka#71): fold in methods from `impl Trait for Enum` /
        // `impl Enum { }`, registered into self.struct_methods by
        // emit_program's pre-pass (order-independent -- the impl block may
        // appear before or after this enum in source). Stored there as full
        // `function(...) { ... }` expressions, so these become `key: value`
        // entries rather than shorthand-method syntax; JS object literals
        // permit freely mixing both forms, same as `__enum`/`__case` above
        // already do.
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
}
