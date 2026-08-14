use super::*;

impl<'a> JsSubsetEmitter<'a> {
    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    pub(super) fn pop_scope(&mut self) {
        if let Some(popped) = self.scopes.pop() {
            // Record variables that were declared only in this block scope
            // (not also in an enclosing scope).  If they are later referenced,
            // we know the user wrote a cross-block variable access that won't
            // work under JS block scoping.
            for name in popped {
                if !self.is_declared(&name) {
                    self.popped_declarations.insert(name);
                }
            }
        }
    }

    pub(super) fn declare_in_scope(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string());
        }
    }

    pub(super) fn emit_param_default_guards(
        &mut self,
        params: &[php_rs::parser::ast::Param<'_>],
        destructure_map: &std::collections::HashMap<String, (String, Vec<String>)>,
    ) -> Result<(), String> {
        for param in params {
            if let Some(default) = param.default {
                let original = self.token_name(param.name);
                let name = destructure_map
                    .get(&original)
                    .map(|(synthetic, _)| synthetic.clone())
                    .unwrap_or(original);
                let default_js = self.emit_expr(default)?;
                self.body.push_str(&format!(
                    "if ({} === undefined) {{ {} = {}; }}\n",
                    name, name, default_js
                ));
            }
        }
        Ok(())
    }

    pub(super) fn emit_param_default_guards_inline(
        &mut self,
        params: &[php_rs::parser::ast::Param<'_>],
        destructure_map: &std::collections::HashMap<String, (String, Vec<String>)>,
    ) -> Result<String, String> {
        let mut out = String::new();
        for param in params {
            if let Some(default) = param.default {
                let original = self.token_name(param.name);
                let name = destructure_map
                    .get(&original)
                    .map(|(synthetic, _)| synthetic.clone())
                    .unwrap_or(original);
                let default_js = self.emit_expr(default)?;
                out.push_str(&format!(
                    "if ({} === undefined) {{ {} = {}; }}\n",
                    name, name, default_js
                ));
            }
        }
        Ok(out)
    }

    /// Detect parser-generated destructuring prologue assignments so the
    /// emitter can skip them when it has already emitted explicit `const`
    /// bindings for a synthetic props parameter.
    pub(super) fn is_param_pattern_prologue(
        &self,
        stmt: &php_rs::parser::ast::Stmt<'_>,
        original_names: &[String],
    ) -> bool {
        let php_rs::parser::ast::Stmt::Expression { expr, .. } = stmt else {
            return false;
        };
        let php_rs::parser::ast::Expr::Assign { expr: rhs, .. } = **expr else {
            return false;
        };
        let target = match rhs {
            php_rs::parser::ast::Expr::PropertyFetch { target, .. } => target,
            php_rs::parser::ast::Expr::ArrayDimFetch { array, .. } => array,
            _ => return false,
        };
        let php_rs::parser::ast::Expr::Variable { name, .. } = *target else {
            return false;
        };
        let target_name = self.span_name(*name);
        original_names.contains(&target_name)
    }

    pub(super) fn is_declared(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.contains(name) {
                return true;
            }
        }
        false
    }

    /// Like `is_declared` but only checks scopes within the current function
    /// body (i.e., scopes at or above the function entry scope index).  This
    /// ensures PHP-style function-local variables always get `let` declarations,
    /// even when an outer (module-level) import or variable has the same name.
    pub(super) fn is_declared_in_current_function(&self, name: &str) -> bool {
        let start = self.function_scope_entry.last().copied().unwrap_or(0);
        for scope in self.scopes[start..].iter().rev() {
            if scope.contains(name) {
                return true;
            }
        }
        false
    }
}
