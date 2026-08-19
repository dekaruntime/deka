use super::*;

impl<'a> JsSubsetEmitter<'a> {
    /// Try to interpret a frontmatter template line as a JSX element and emit it
    /// as a `jsx(...)` / `jsxs(...)` expression. Returns `None` for lines that
    /// should fall back to literal text emission.
    pub(super) fn try_emit_template_jsx(&mut self, line: &str) -> Option<String> {
        let s = line.trim();
        if s.is_empty() || !s.starts_with('<') {
            return None;
        }

        let bytes = s.as_bytes();
        let mut i = 1usize;

        // Parse tag name.
        let tag_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == tag_start {
            return None;
        }
        let tag_name = &s[tag_start..i];
        let is_component = tag_name
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);

        // Parse attributes.
        let mut attrs: Vec<String> = Vec::new();
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] == b'/' || bytes[i] == b'>' {
                break;
            }
            let name_start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'_'
                    || bytes[i] == b'-')
            {
                i += 1;
            }
            if i == name_start {
                return None;
            }
            let attr_name = &s[name_start..i];
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'=' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                    let quote = bytes[i];
                    i += 1;
                    let val_start = i;
                    while i < bytes.len() && bytes[i] != quote {
                        i += 1;
                    }
                    let value = &s[val_start..i];
                    attrs.push(format!("{}: {}", json_string(attr_name), json_string(value)));
                    if i < bytes.len() {
                        i += 1;
                    }
                } else if i < bytes.len() && bytes[i] == b'{' {
                    i += 1;
                    let expr_start = i;
                    let mut depth = 1usize;
                    while i < bytes.len() && depth > 0 {
                        if bytes[i] == b'{' {
                            depth += 1;
                        } else if bytes[i] == b'}' {
                            depth -= 1;
                        }
                        i += 1;
                    }
                    let expr = &s[expr_start..i.saturating_sub(1)];
                    let js_expr = expr.trim().replace('$', "");
                    attrs.push(format!("{}: {}", json_string(attr_name), js_expr));
                } else {
                    return None;
                }
            } else {
                attrs.push(format!("{}: true", json_string(attr_name)));
            }
        }

        let self_closing = if i < bytes.len() && bytes[i] == b'/' {
            i += 1;
            true
        } else {
            false
        };
        if i >= bytes.len() || bytes[i] != b'>' {
            return None;
        }
        i += 1;

        let mut children: Vec<String> = Vec::new();
        if !self_closing {
            let content_start = i;
            let close_tag = format!("</{}>", tag_name);
            let close_pos = s[content_start..].find(&close_tag)? + content_start;
            let content = &s[content_start..close_pos];

            let mut j = 0usize;
            let cbytes = content.as_bytes();
            while j < content.len() {
                if cbytes[j] == b'{' {
                    let start = j;
                    j += 1;
                    while j < content.len() && cbytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= content.len() || cbytes[j] != b'$' {
                        j = start;
                    } else {
                        j += 1;
                        while j < content.len() && cbytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        let expr_start = j;
                        while j < content.len()
                            && (cbytes[j].is_ascii_alphanumeric()
                                || cbytes[j] == b'_'
                                || cbytes[j] == b'.')
                        {
                            j += 1;
                        }
                        let expr_end = j;
                        while j < content.len() && cbytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if expr_end > expr_start && j < content.len() && cbytes[j] == b'}' {
                            let expr = &content[expr_start..expr_end];
                            let js_expr = expr.replace('$', "");
                            children.push(js_expr);
                            j += 1;
                            continue;
                        }
                        j = start;
                    }
                }
                let text_start = j;
                while j < content.len() && cbytes[j] != b'{' {
                    j += 1;
                }
                let text = &content[text_start..j];
                if !text.is_empty() {
                    children.push(json_string(text));
                }
            }
        }

        self.uses_jsx_runtime = true;
        if !children.is_empty() {
            if children.len() == 1 {
                attrs.push(format!("\"children\": {}", children[0]));
            } else {
                attrs.push(format!("\"children\": [{}]", children.join(", ")));
            }
        }

        let tag_expr = if is_component {
            tag_name.to_string()
        } else {
            json_string(tag_name)
        };
        let props_expr = format!("{{{}}}", attrs.join(", "));
        let fn_name = if children.len() > 1 { "jsxs" } else { "jsx" };
        Some(format!("deka.ui.{}({}, {})", fn_name, tag_expr, props_expr))
    }

    pub(super) fn emit_jsx(
        &mut self,
        name: Option<php_rs::parser::ast::Name<'_>>,
        attributes: &[php_rs::parser::ast::JsxAttribute<'_>],
        children: &[JsxChild<'_>],
    ) -> Result<String, String> {
        self.uses_jsx_runtime = true;

        let mut props = Vec::new();
        for attr in attributes {
            // DekaScript JSX spread attribute: `{ ...props }`
            if let Some(Expr::Spread { expr, .. }) = attr.value {
                let spread_expr = self.emit_expr(expr)?;
                props.push(format!("...{}", spread_expr));
                continue;
            }
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
            None => "deka.ui.Fragment".to_string(),
        };

        let props_expr = format!("{{{}}}", props.join(", "));
        let fn_name = if child_values.len() > 1 {
            "jsxs"
        } else {
            "jsx"
        };

        Ok(format!("deka.ui.{}({}, {})", fn_name, tag_expr, props_expr))
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
        let defaults =
            self.emit_param_default_guards_inline(params, &std::collections::HashMap::new())?;
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

    /// Emit the body of a DekaScript receiver method (`fn (p Person) greet() { ... }`).
    /// The receiver variable is declared as a real parameter so the method body can
    /// reference it by name; this is distinct from `impl` block methods where `self`
    /// is erased and bound to JS `this`.
    pub(super) fn emit_receiver_method_block(
        &mut self,
        receiver: &Receiver<'_>,
        params: &[php_rs::parser::ast::Param<'_>],
        stmts: &[StmtId<'_>],
    ) -> Result<String, String> {
        let saved = std::mem::take(&mut self.body);
        self.push_scope();
        self.function_scope_entry.push(self.scopes.len() - 1);
        self.declare_in_scope(&self.token_name(receiver.var));
        for param in params {
            self.declare_in_scope(&self.token_name(param.name));
        }
        let defaults =
            self.emit_param_default_guards_inline(params, &std::collections::HashMap::new())?;
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
