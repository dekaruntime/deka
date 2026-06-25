use super::*;

impl<'a> JsSubsetEmitter<'a> {
    pub(super) fn emit_jsx(
        &mut self,
        name: Option<php_rs::parser::ast::Name<'_>>,
        attributes: &[php_rs::parser::ast::JsxAttribute<'_>],
        children: &[JsxChild<'_>],
    ) -> Result<String, String> {
        self.uses_jsx_runtime = true;

        let mut props = Vec::new();
        for attr in attributes {
            let key = self.token_text(attr.name);
            let value = if let Some(expr) = attr.value {
                self.emit_expr(expr)?
            } else {
                "true".to_string()
            };
            props.push(format!("{}: {}", json_string(&key), value));
        }

        let mut child_values = Vec::new();
        for child in children {
            match child {
                JsxChild::Text(span) => {
                    if let Some(text) = self.normalize_jsx_text(*span) {
                        child_values.push(json_string(&text));
                    }
                }
                JsxChild::Expr(expr) => child_values.push(self.emit_expr(*expr)?),
            }
        }

        if !child_values.is_empty() {
            if child_values.len() == 1 {
                props.push(format!("\"children\": {}", child_values[0]));
            } else {
                props.push(format!("\"children\": [{}]", child_values.join(", ")));
            }
        }

        let tag_expr = match name {
            Some(n) => {
                let raw = self.span_bytes(n.span);
                let trimmed = raw.strip_prefix(b"\\").unwrap_or(raw);
                let raw_text = String::from_utf8_lossy(trimmed).to_string();
                let last = raw_text.rsplit('\\').next().unwrap_or(raw_text.as_str());
                let is_component = last
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false);
                if is_component {
                    last.to_string()
                } else {
                    json_string(last)
                }
            }
            None => json_string("__fragment__"),
        };

        let props_expr = format!("{{{}}}", props.join(", "));
        let fn_name = if child_values.len() > 1 {
            "jsxs"
        } else {
            "jsx"
        };

        Ok(format!("{}({}, {})", fn_name, tag_expr, props_expr))
    }

    pub(super) fn emit_stmt_block_inline(
        &mut self,
        stmts: &[StmtId<'_>],
    ) -> Result<String, String> {
        let saved = std::mem::take(&mut self.body);
        self.push_scope();
        for stmt in stmts {
            self.emit_stmt(*stmt)?;
        }
        self.pop_scope();
        let block = std::mem::take(&mut self.body);
        self.body = saved;
        Ok(block)
    }

    pub(super) fn emit_method_block(
        &mut self,
        params: &[php_rs::parser::ast::Param<'_>],
        stmts: &[StmtId<'_>],
    ) -> Result<String, String> {
        let saved = std::mem::take(&mut self.body);
        self.push_scope();
        self.function_scope_entry.push(self.scopes.len() - 1);
        for param in params {
            self.declare_in_scope(&self.token_name(param.name));
        }
        let defaults = self.emit_param_default_guards_inline(params)?;
        if !defaults.is_empty() {
            self.body.push_str(&defaults);
        }
        for stmt in stmts {
            self.emit_stmt(*stmt)?;
        }
        self.function_scope_entry.pop();
        self.pop_scope();
        let block = std::mem::take(&mut self.body);
        self.body = saved;
        Ok(block)
    }

    pub(super) fn emit_expr_list(&mut self, exprs: &[ExprId<'_>]) -> Result<String, String> {
        if exprs.is_empty() {
            return Ok(String::new());
        }
        let mut out = Vec::with_capacity(exprs.len());
        for expr in exprs {
            out.push(self.emit_expr(*expr)?);
        }
        Ok(out.join(", "))
    }

    pub(super) fn emit_for_init(&mut self, exprs: &[ExprId<'_>]) -> Result<String, String> {
        if exprs.is_empty() {
            return Ok(String::new());
        }

        let mut parts = Vec::with_capacity(exprs.len());
        let mut all_new_assignments = true;

        for expr in exprs {
            if let Some((name, rhs)) = self.assignment_to_named_var(*expr)? {
                if self.is_declared(&name) {
                    all_new_assignments = false;
                    parts.push(format!("{} = {}", name, rhs));
                } else {
                    self.declare_in_scope(&name);
                    parts.push(format!("{} = {}", name, rhs));
                }
            } else {
                all_new_assignments = false;
                parts.push(self.emit_expr(*expr)?);
            }
        }

        if all_new_assignments {
            Ok(format!("let {}", parts.join(", ")))
        } else {
            Ok(parts.join(", "))
        }
    }

    pub(super) fn extract_var_name(&self, expr: ExprId<'_>) -> Result<String, String> {
        match expr {
            Expr::Variable { name, .. } => Ok(self.span_name(*name)),
            _ => Err("foreach key/value target must be a variable in subset emitter".to_string()),
        }
    }

    pub(super) fn extract_static_var_name(&self, expr: ExprId<'_>) -> Option<String> {
        match expr {
            Expr::Variable { name, .. } => Some(self.span_name(*name)),
            Expr::IndirectVariable { name, .. } => match *name {
                Expr::Variable { name, .. } => Some(self.span_name(*name)),
                _ => Some(self.span_name(name.span())),
            },
            _ => Some(self.span_name(expr.span())),
        }
    }
}
