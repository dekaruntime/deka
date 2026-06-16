use super::*;

impl<'a> JsSubsetEmitter<'a> {
    pub(super) fn emit_call_args(
        &mut self,
        args: &[php_rs::parser::ast::Arg<'_>],
    ) -> Result<String, String> {
        if args.iter().all(|a| a.name.is_none() && !a.unpack) {
            let mut rendered = Vec::with_capacity(args.len());
            for arg in args {
                rendered.push(self.emit_expr(arg.value)?);
            }
            return Ok(rendered.join(", "));
        }

        if args.iter().all(|a| a.name.is_some() && !a.unpack) {
            let mut entries = Vec::with_capacity(args.len());
            for arg in args {
                let name = arg
                    .name
                    .map(|tok| self.sanitize_name(&self.token_text(tok)))
                    .ok_or_else(|| {
                        "mixed positional/named arguments are not supported in subset emitter"
                            .to_string()
                    })?;
                let value = self.emit_expr(arg.value)?;
                entries.push(format!("{}: {}", json_string(&name), value));
            }
            return Ok(format!("{{{}}}", entries.join(", ")));
        }

        Err(
            "mixed positional/named/unpack call arguments are not supported in subset emitter"
                .to_string(),
        )
    }

    pub(super) fn emit_match_expr(
        &mut self,
        condition: ExprId<'_>,
        arms: &[php_rs::parser::ast::MatchArm<'_>],
    ) -> Result<String, String> {
        let condition_js = self.emit_expr(condition)?;
        let mut rendered = String::new();

        for arm in arms.iter().rev() {
            let arm_expr = self.emit_expr(arm.body)?;
            if arm.conditions.is_none() {
                rendered = arm_expr;
                continue;
            }
            let guard = self.emit_match_guard(&condition_js, arm.conditions)?;
            if rendered.is_empty() {
                rendered = format!("({} ? {} : undefined)", guard, arm_expr);
            } else {
                rendered = format!("({} ? {} : {})", guard, arm_expr, rendered);
            }
        }

        if rendered.is_empty() {
            return Err("match requires at least one arm".to_string());
        }
        Ok(rendered)
    }

    pub(super) fn emit_match_guard(
        &mut self,
        condition_js: &str,
        conditions: Option<&[ExprId<'_>]>,
    ) -> Result<String, String> {
        let Some(conditions) = conditions else {
            return Ok("true".to_string());
        };
        if conditions.is_empty() {
            return Ok("false".to_string());
        }

        let mut checks = Vec::with_capacity(conditions.len());
        for cond in conditions {
            if let Some((enum_name, case_name)) = self.enum_case_from_expr(*cond) {
                checks.push(format!(
                    "({} instanceof {} && {}.__case === {})",
                    condition_js,
                    enum_name,
                    condition_js,
                    json_string(&case_name)
                ));
                continue;
            }
            let rhs = self.emit_expr(*cond)?;
            checks.push(format!("({} === {})", condition_js, rhs));
        }
        Ok(format!("({})", checks.join(" || ")))
    }

    pub(super) fn enum_case_from_expr(&self, expr: ExprId<'_>) -> Option<(String, String)> {
        match expr {
            Expr::ClassConstFetch {
                class, constant, ..
            } => {
                let enum_name = self.extract_static_name(*class)?;
                let case_name = self.extract_static_name(*constant)?;
                let cases = self.enum_cases.get(&enum_name)?;
                if cases.iter().any(|c| c.name == case_name) {
                    Some((enum_name, case_name))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(super) fn extract_static_name(&self, expr: ExprId<'_>) -> Option<String> {
        match expr {
            Expr::Variable { name, .. } => Some(self.span_name(*name)),
            Expr::String { value, .. } => {
                let mut bytes: &[u8] = value;
                if bytes.len() >= 2
                    && ((bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
                        || (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"'))
                {
                    bytes = &bytes[1..bytes.len() - 1];
                }
                Some(String::from_utf8_lossy(bytes).to_string())
            }
            _ => None,
        }
    }

    pub(super) fn emit_static_array_key(&self, expr: ExprId<'_>) -> Result<String, String> {
        match expr {
            Expr::String { value, .. } => {
                let mut bytes: &[u8] = value;
                if bytes.len() >= 2
                    && ((bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
                        || (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"'))
                {
                    bytes = &bytes[1..bytes.len() - 1];
                }
                Ok(String::from_utf8_lossy(bytes).to_string())
            }
            Expr::Integer { value, .. } => Ok(String::from_utf8_lossy(value).to_string()),
            Expr::Variable { name, .. } => Ok(self.span_name(*name)),
            _ => {
                Err("array key must be static string/int/identifier in subset emitter".to_string())
            }
        }
    }

    pub(super) fn emit_assignable_expr(&mut self, expr: ExprId<'_>) -> Result<String, String> {
        match expr {
            Expr::Variable { name, .. } => Ok(self.span_name(*name)),
            Expr::DotAccess {
                target, property, ..
            } => {
                let target_js = self.emit_expr(*target)?;
                let prop = self.token_text(property);
                Ok(format!("{}.{}", target_js, prop))
            }
            Expr::PropertyFetch {
                target, property, ..
            } => {
                let target_js = self.emit_expr(*target)?;
                match *property {
                    Expr::Variable { name, .. } => {
                        Ok(format!("{}.{}", target_js, self.span_name(*name)))
                    }
                    _ => {
                        let prop = self.emit_expr(*property)?;
                        Ok(format!("{}[{}]", target_js, prop))
                    }
                }
            }
            Expr::ArrayDimFetch { array, dim, .. } => {
                let array_js = self.emit_expr(*array)?;
                if let Some(dim) = dim {
                    let dim_js = self.emit_expr(*dim)?;
                    Ok(format!("{}[{}]", array_js, dim_js))
                } else {
                    Err(
                        "append array access is not supported in assignable expressions"
                            .to_string(),
                    )
                }
            }
            _ => Err("assignment target is not supported in subset emitter".to_string()),
        }
    }

    pub(super) fn emit_assignment_target(
        &mut self,
        expr: ExprId<'_>,
    ) -> Result<AssignmentTarget, String> {
        match expr {
            Expr::ArrayDimFetch { array, dim, .. } => {
                let array_js = self.emit_expr(*array)?;
                if let Some(dim) = dim {
                    let dim_js = self.emit_expr(*dim)?;
                    Ok(AssignmentTarget::Direct(format!(
                        "{}[{}]",
                        array_js, dim_js
                    )))
                } else {
                    Ok(AssignmentTarget::Append(array_js))
                }
            }
            _ => Ok(AssignmentTarget::Direct(self.emit_assignable_expr(expr)?)),
        }
    }

    pub(super) fn assignment_to_named_var(
        &mut self,
        expr: ExprId<'_>,
    ) -> Result<Option<(String, String)>, String> {
        match expr {
            Expr::Assign { var, expr, .. } | Expr::AssignRef { var, expr, .. } => {
                if let Expr::Variable { name, .. } = *var {
                    let js_name = self.span_name(*name);
                    let rhs = self.emit_expr(*expr)?;
                    Ok(Some((js_name, rhs)))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    pub(super) fn name_last_segment(&self, name: php_rs::parser::ast::Name<'_>) -> String {
        if let Some(last) = name.parts.last() {
            self.token_text(last).trim_start_matches('\\').to_string()
        } else {
            let raw = String::from_utf8_lossy(self.span_bytes(name.span)).to_string();
            raw.rsplit('\\').next().unwrap_or(raw.as_str()).to_string()
        }
    }
}
