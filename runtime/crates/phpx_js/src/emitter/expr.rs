use super::*;

impl<'a> JsSubsetEmitter<'a> {
    pub(super) fn emit_expr(&mut self, expr: ExprId<'_>) -> Result<String, String> {
        match expr {
            Expr::Variable { name, .. } => {
                let ident = self.span_name(*name);
                if ident == "this" {
                    return Ok("this".to_string());
                }
                if self.is_declared(&ident) {
                    Ok(ident)
                } else {
                    // Phase 3: detect cross-block variable access.  If the
                    // variable was `let`-declared inside a block scope that has
                    // since been popped, the user is reading a variable that
                    // won't be visible under JS block scoping rules.
                    if self.popped_declarations.contains(&ident) {
                        self.warnings.push(format!(
                            "Variable `${}` is first assigned inside a block. \
                             Under PHPX scoping rules, it won't be visible outside. \
                             Declare it at function level if needed.",
                            ident
                        ));
                    }
                    Ok(format!("globalThis.{}", ident))
                }
            }
            Expr::Integer { value, .. } | Expr::Float { value, .. } => {
                Ok(String::from_utf8_lossy(value).to_string())
            }
            Expr::Boolean { value, .. } => Ok(if *value { "true" } else { "false" }.to_string()),
            Expr::Null { .. } => Ok("null".to_string()),
            Expr::String { value, .. } => Ok(self.encode_php_string_literal(value)),
            Expr::Unary { op, expr, .. } => {
                let value = self.emit_expr(*expr)?;
                let js_op = match op {
                    UnaryOp::Plus => "+",
                    UnaryOp::Minus => "-",
                    UnaryOp::Not => "!",
                    UnaryOp::BitNot => "~",
                    UnaryOp::PreInc => "++",
                    UnaryOp::PreDec => "--",
                    _ => {
                        return Err(format!(
                            "unsupported unary operator in subset emitter: {:?}",
                            op
                        ));
                    }
                };
                Ok(format!("({}{})", js_op, value))
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                let lhs = self.emit_expr(*left)?;
                if matches!(op, BinaryOp::Instanceof) {
                    if let Expr::Variable { name, .. } = right {
                        let ident = self.span_name(*name);
                        if self.struct_names.contains(&ident) {
                            return Ok(format!(
                                "(globalThis.__phpx_is_struct({}, {}))",
                                lhs,
                                json_string(&ident)
                            ));
                        }
                    }
                }
                let rhs = self.emit_expr(*right)?;
                let js_op = match op {
                    BinaryOp::Plus => "+",
                    BinaryOp::Minus => "-",
                    BinaryOp::Mul => "*",
                    BinaryOp::Div => "/",
                    BinaryOp::Mod => "%",
                    BinaryOp::Pow => "**",
                    BinaryOp::ShiftLeft => "<<",
                    BinaryOp::ShiftRight => ">>",
                    BinaryOp::BitAnd => "&",
                    BinaryOp::BitOr => "|",
                    BinaryOp::BitXor => "^",
                    BinaryOp::Concat => "+",
                    BinaryOp::Instanceof => "instanceof",
                    BinaryOp::Eq | BinaryOp::EqEq => "===",
                    BinaryOp::EqEqEq => "===",
                    BinaryOp::NotEq => "!==",
                    BinaryOp::NotEqEq => "!==",
                    BinaryOp::Lt => "<",
                    BinaryOp::LtEq => "<=",
                    BinaryOp::Gt => ">",
                    BinaryOp::GtEq => ">=",
                    BinaryOp::And | BinaryOp::LogicalAnd => "&&",
                    BinaryOp::Or | BinaryOp::LogicalOr => "||",
                    BinaryOp::Coalesce => "??",
                    _ => {
                        return Err(format!(
                            "unsupported binary operator in subset emitter: {:?}",
                            op
                        ));
                    }
                };
                Ok(format!("({} {} {})", lhs, js_op, rhs))
            }
            Expr::ArrowFunction { params, expr, .. } => {
                let mut names = Vec::with_capacity(params.len());
                for param in *params {
                    names.push(self.token_name(param.name));
                }
                self.push_scope();
                for param in *params {
                    self.declare_in_scope(&self.token_name(param.name));
                }
                let body = self.emit_expr(*expr)?;
                self.pop_scope();
                Ok(format!("({}) => {}", names.join(", "), body))
            }
            Expr::Closure {
                params,
                body,
                is_async,
                uses,
                ..
            } => {
                let mut names = Vec::with_capacity(params.len());
                for param in *params {
                    names.push(self.token_name(param.name));
                }
                let block = self.emit_method_block(params, body)?;
                let async_kw = if *is_async { "async " } else { "" };
                let fn_expr = format!(
                    "{}function({}) {{\n{} }}",
                    async_kw,
                    names.join(", "),
                    block
                );
                if uses.is_empty() {
                    return Ok(fn_expr);
                }

                let mut cap_params = Vec::new();
                let mut cap_bindings = Vec::new();
                let mut cap_args = Vec::new();
                for item in *uses {
                    let local = self.token_name(item.var);
                    if local.is_empty() {
                        continue;
                    }
                    let cap_name = format!("__phpx_cap_{}", local);
                    cap_params.push(cap_name.clone());
                    cap_bindings.push(format!("const {} = {};", local, cap_name));
                    if self.is_declared(&local) {
                        cap_args.push(local);
                    } else {
                        cap_args.push(format!("globalThis.{}", local));
                    }
                }

                Ok(format!(
                    "(({}) => {{\n{}\nreturn {};\n}})({})",
                    cap_params.join(", "),
                    cap_bindings.join("\n"),
                    fn_expr,
                    cap_args.join(", ")
                ))
            }
            Expr::Call { func, args, .. } => {
                if let Expr::Variable { name, .. } = func {
                    let ident = self.span_name(*name);
                    if ident == "func_num_args" {
                        return Ok("globalThis.__phpx_func_num_args(arguments)".to_string());
                    }
                    if ident == "func_get_args" {
                        return Ok("globalThis.__phpx_func_get_args(arguments)".to_string());
                    }
                    if ident == "func_get_arg" {
                        let args_js = self.emit_call_args(args)?;
                        if args_js.is_empty() {
                            return Ok("globalThis.__phpx_func_get_arg(arguments, 0)".to_string());
                        }
                        return Ok(format!(
                            "globalThis.__phpx_func_get_arg(arguments, {})",
                            args_js
                        ));
                    }
                    // --- Phase 2: compile-time rewrites for class (a) builtins ---
                    if !self.is_declared(&ident) {
                        if let Some(rewritten) = self.try_rewrite_builtin(&ident, args)? {
                            return Ok(rewritten);
                        }
                    }
                }
                let callee = self.emit_expr(*func)?;
                let args_js = self.emit_call_args(args)?;
                Ok(format!("{}({})", callee, args_js))
            }
            Expr::Assign { var, expr, .. } => {
                let rhs = self.emit_expr(*expr)?;
                match self.emit_assignment_target(*var)? {
                    AssignmentTarget::Direct(target) => Ok(format!("({} = {})", target, rhs)),
                    AssignmentTarget::Append(array) => Ok(format!(
                        "(() => {{ const __arr = {}; const __val = {}; __arr.push(__val); return __val; }})()",
                        array, rhs
                    )),
                }
            }
            Expr::AssignRef { var, expr, .. } => {
                let rhs = self.emit_expr(*expr)?;
                match self.emit_assignment_target(*var)? {
                    AssignmentTarget::Direct(target) => Ok(format!("({} = {})", target, rhs)),
                    AssignmentTarget::Append(array) => Ok(format!(
                        "(() => {{ const __arr = {}; const __val = {}; __arr.push(__val); return __val; }})()",
                        array, rhs
                    )),
                }
            }
            Expr::AssignOp { var, op, expr, .. } => {
                let rhs = self.emit_expr(*expr)?;
                let js_op = match op {
                    php_rs::parser::ast::AssignOp::Plus => "+=",
                    php_rs::parser::ast::AssignOp::Minus => "-=",
                    php_rs::parser::ast::AssignOp::Mul => "*=",
                    php_rs::parser::ast::AssignOp::Div => "/=",
                    php_rs::parser::ast::AssignOp::Mod => "%=",
                    php_rs::parser::ast::AssignOp::Concat => "+=",
                    php_rs::parser::ast::AssignOp::BitAnd => "&=",
                    php_rs::parser::ast::AssignOp::BitOr => "|=",
                    php_rs::parser::ast::AssignOp::BitXor => "^=",
                    php_rs::parser::ast::AssignOp::ShiftLeft => "<<=",
                    php_rs::parser::ast::AssignOp::ShiftRight => ">>=",
                    php_rs::parser::ast::AssignOp::Pow => "**=",
                    php_rs::parser::ast::AssignOp::Coalesce => "??=",
                };
                match self.emit_assignment_target(*var)? {
                    AssignmentTarget::Direct(target) => {
                        Ok(format!("({} {} {})", target, js_op, rhs))
                    }
                    AssignmentTarget::Append(_) => Err(
                        "assign-op on append array access is not supported in subset emitter"
                            .to_string(),
                    ),
                }
            }
            Expr::PropertyFetch {
                target, property, ..
            } => {
                let target_js = self.emit_expr(*target)?;
                match *property {
                    Expr::Variable { name, .. } => {
                        let prop = self.span_name(*name);
                        Ok(format!("{}.{}", target_js, prop))
                    }
                    _ => {
                        let prop = self.emit_expr(*property)?;
                        Ok(format!("{}[{}]", target_js, prop))
                    }
                }
            }
            Expr::MethodCall {
                target,
                method,
                args,
                ..
            } => {
                let target_js = self.emit_expr(*target)?;
                let method_name = match *method {
                    Expr::Variable { name, .. } => self.span_name(*name),
                    _ => {
                        return Err(
                            "dynamic method calls are not supported in subset emitter".to_string()
                        );
                    }
                };
                let args_js = self.emit_call_args(args)?;
                Ok(format!("{}.{}({})", target_js, method_name, args_js))
            }
            Expr::StaticCall {
                class,
                method,
                args,
                ..
            } => {
                let class_js = self.emit_expr(*class)?;
                let method_name = match *method {
                    Expr::Variable { name, .. } => self.span_name(*name),
                    _ => {
                        return Err(
                            "dynamic static method calls are not supported in subset emitter"
                                .to_string(),
                        );
                    }
                };
                let args_js = self.emit_call_args(args)?;
                Ok(format!("{}.{}({})", class_js, method_name, args_js))
            }
            Expr::ClassConstFetch {
                class, constant, ..
            } => {
                let class_js = self.emit_expr(*class)?;
                let const_name = match *constant {
                    Expr::Variable { name, .. } => self.span_name(*name),
                    _ => {
                        return Err(
                            "dynamic class constant access is not supported in subset emitter"
                                .to_string(),
                        );
                    }
                };
                Ok(format!("{}.{}", class_js, const_name))
            }
            Expr::New { class, args, .. } => {
                let class_js = self.emit_expr(*class)?;
                let args_js = self.emit_call_args(args)?;
                Ok(format!("new {}({})", class_js, args_js))
            }
            Expr::NullsafePropertyFetch {
                target, property, ..
            } => {
                let target_js = self.emit_expr(*target)?;
                match *property {
                    Expr::Variable { name, .. } => {
                        let prop = self.span_name(*name);
                        Ok(format!("({})?.{}", target_js, prop))
                    }
                    _ => {
                        let prop = self.emit_expr(*property)?;
                        Ok(format!("({})?.[{}]", target_js, prop))
                    }
                }
            }
            Expr::NullsafeMethodCall {
                target,
                method,
                args,
                ..
            } => {
                let target_js = self.emit_expr(*target)?;
                let method_name = match *method {
                    Expr::Variable { name, .. } => self.span_name(*name),
                    _ => {
                        return Err(
                            "dynamic nullsafe method calls are not supported in subset emitter"
                                .to_string(),
                        );
                    }
                };
                let args_js = self.emit_call_args(args)?;
                Ok(format!("({})?.{}({})", target_js, method_name, args_js))
            }
            Expr::DotAccess {
                target, property, ..
            } => {
                let target_js = self.emit_expr(*target)?;
                let prop = self.token_text(property);
                if let Some(raw) = prop.strip_prefix('$') {
                    let name = self.sanitize_name(raw);
                    let value = if self.is_declared(&name) {
                        name
                    } else {
                        format!("globalThis.{}", name)
                    };
                    Ok(format!("{}[{}]", target_js, value))
                } else {
                    Ok(format!("{}.{}", target_js, prop))
                }
            }
            Expr::ArrayDimFetch { array, dim, .. } => {
                let array_js = self.emit_expr(*array)?;
                if let Some(dim) = dim {
                    let dim_js = self.emit_expr(*dim)?;
                    Ok(format!("{}[{}]", array_js, dim_js))
                } else {
                    Err("append array access is not supported in subset emitter".to_string())
                }
            }
            Expr::Array { items, .. } => {
                if !items.iter().all(|item| !item.by_ref && !item.unpack) {
                    return Err(
                        "mixed or complex array items are not supported in subset emitter"
                            .to_string(),
                    );
                }

                let mut has_keys = false;
                let mut has_no_keys = false;
                for item in *items {
                    if item.key.is_some() {
                        has_keys = true;
                    } else {
                        has_no_keys = true;
                    }
                }

                if !has_keys {
                    let mut values = Vec::new();
                    for item in *items {
                        values.push(self.emit_expr(item.value)?);
                    }
                    return Ok(format!("[{}]", values.join(", ")));
                }

                if !has_no_keys {
                    let mut entries = Vec::new();
                    let mut all_static = true;
                    for item in *items {
                        let key_expr = item
                            .key
                            .ok_or_else(|| "keyed array expected key".to_string())?;
                        let key = match self.emit_static_array_key(key_expr) {
                            Ok(value) => value,
                            Err(_) => {
                                all_static = false;
                                break;
                            }
                        };
                        let value = self.emit_expr(item.value)?;
                        entries.push(format!("{}: {}", json_string(&key), value));
                    }
                    if all_static {
                        return Ok(format!("{{{}}}", entries.join(", ")));
                    }
                }

                let mut out = String::new();
                out.push_str("(() => { const __out = []; let __idx = 0;\n");
                let mut key_idx = 0;
                for item in *items {
                    if let Some(key_expr) = item.key {
                        let key_js = self.emit_expr(key_expr)?;
                        let value_js = self.emit_expr(item.value)?;
                        out.push_str(&format!(
                            "const __key{key_idx} = {key_js}; __out[__key{key_idx}] = {value_js}; if (typeof __key{key_idx} === 'number' && Number.isInteger(__key{key_idx})) {{ __idx = Math.max(__idx, __key{key_idx} + 1); }}\n",
                        ));
                        key_idx += 1;
                    } else {
                        let value_js = self.emit_expr(item.value)?;
                        out.push_str(&format!("__out[__idx++] = {};\n", value_js));
                    }
                }
                out.push_str("return __out; })()");
                Ok(out)
            }
            Expr::ObjectLiteral { items, .. } => {
                let mut entries = Vec::new();
                for item in *items {
                    let key = match item.key {
                        ObjectKey::Ident(tok) => self.token_text(tok),
                        // String-literal keys carry their quote delimiters in
                        // the source span (e.g. `'content-type'`). Strip the
                        // outer quotes and decode escapes so the JS emitter
                        // receives the logical key name; `json_string` below
                        // will re-quote it correctly.
                        ObjectKey::String(tok) => decode_string_key(&self.token_text(tok)),
                    };
                    let value = self.emit_expr(item.value)?;
                    entries.push(format!("{}: {}", json_string(&key), value));
                }
                Ok(format!("{{{}}}", entries.join(", ")))
            }
            Expr::StructLiteral { name, fields, .. } => {
                let struct_name = self.name_last_segment(*name);
                let mut entries = Vec::new();
                entries.push(format!(
                    "{}: {}",
                    json_string("__struct"),
                    json_string(&struct_name)
                ));
                for field in *fields {
                    let key = self
                        .token_text(field.name)
                        .trim_start_matches('$')
                        .to_string();
                    let value = self.emit_expr(field.value)?;
                    entries.push(format!("{}: {}", json_string(&key), value));
                }
                Ok(format!(
                    "(() => {{ const __obj = {{{}}}; const __m = globalThis.__phpxStructMethods ? globalThis.__phpxStructMethods[{}] : null; if (__m) Object.assign(__obj, __m); return __obj; }})()",
                    entries.join(", "),
                    json_string(&struct_name)
                ))
            }
            Expr::JsxElement {
                name,
                attributes,
                children,
                ..
            } => self.emit_jsx(Some(*name), attributes, children),
            Expr::JsxFragment { children, .. } => self.emit_jsx(None, &[], children),
            Expr::Cast { kind, expr, .. } => {
                let value = self.emit_expr(*expr)?;
                let lowered = match kind {
                    php_rs::parser::ast::CastKind::Int => format!("Number.parseInt({}, 10)", value),
                    php_rs::parser::ast::CastKind::Float => format!("Number({})", value),
                    php_rs::parser::ast::CastKind::String => format!("String({})", value),
                    php_rs::parser::ast::CastKind::Bool => format!("Boolean({})", value),
                    php_rs::parser::ast::CastKind::Array => {
                        format!(
                            "Array.isArray({0}) ? {0} : ({0} && typeof {0} === 'object' ? Object.fromEntries(Object.entries({0})) : [{0}])",
                            value
                        )
                    }
                    php_rs::parser::ast::CastKind::Object => format!("({})", value),
                    _ => {
                        return Err(format!(
                            "unsupported cast kind in subset emitter: {:?}",
                            kind
                        ));
                    }
                };
                Ok(lowered)
            }
            Expr::Isset { vars, .. } => {
                if vars.is_empty() {
                    return Ok("false".to_string());
                }
                let mut checks = Vec::with_capacity(vars.len());
                for var in *vars {
                    let value = self.emit_expr(*var)?;
                    checks.push(format!("({0} !== undefined && {0} !== null)", value));
                }
                Ok(format!("({})", checks.join(" && ")))
            }
            Expr::PostInc { var, .. } => {
                let target = self.emit_assignable_expr(*var)?;
                Ok(format!("({}++)", target))
            }
            Expr::PostDec { var, .. } => {
                let target = self.emit_assignable_expr(*var)?;
                Ok(format!("({}--)", target))
            }
            Expr::Empty { expr, .. } => {
                let value = self.emit_expr(*expr)?;
                Ok(format!("(!({}))", value))
            }
            Expr::Print { expr, .. } => {
                let value = self.emit_expr(*expr)?;
                Ok(format!(
                    "(globalThis.__dekaPrint ? globalThis.__dekaPrint({}) : console.log({}), undefined)",
                    value, value
                ))
            }
            Expr::Await { expr, .. } => {
                let value = self.emit_expr(*expr)?;
                Ok(format!("(await {})", value))
            }
            Expr::Eval { expr, .. } => {
                let value = self.emit_expr(*expr)?;
                Ok(format!("eval({})", value))
            }
            Expr::Clone { expr, .. } => {
                let value = self.emit_expr(*expr)?;
                Ok(format!("structuredClone({})", value))
            }
            Expr::Die { expr, .. } | Expr::Exit { expr, .. } => {
                if let Some(value) = expr {
                    let rendered = self.emit_expr(*value)?;
                    Ok(format!(
                        "(() => {{ throw new Error(String({})); }})()",
                        rendered
                    ))
                } else {
                    Ok("(() => { throw new Error(\"exit\"); })()".to_string())
                }
            }
            Expr::ShellExec { .. } => {
                Err("shell execution is not supported in JS subset emitter".to_string())
            }
            Expr::Yield { .. } => {
                Err("yield expressions are not supported in JS subset emitter".to_string())
            }
            Expr::AnonymousClass { .. } => {
                Err("anonymous classes are not supported in JS subset emitter".to_string())
            }
            Expr::VariadicPlaceholder { .. } => {
                Err("variadic placeholder is not supported in JS subset emitter".to_string())
            }
            Expr::Cql {
                name,
                cypher,
                params,
                span,
            } => {
                let cypher_text = std::str::from_utf8(cypher.as_str(self.source))
                    .unwrap_or("")
                    .trim();
                let name_text = std::str::from_utf8(name.text(self.source)).unwrap_or("_cql");

                // Build the params object: { param_name: param_name, ... }
                let mut param_entries = Vec::new();
                for p in params.iter() {
                    let pname = std::str::from_utf8(p.name).unwrap_or("_");
                    param_entries.push(format!("{0}: {0}", pname));
                }
                let params_obj = if param_entries.is_empty() {
                    "{}".to_string()
                } else {
                    format!("{{ {} }}", param_entries.join(", "))
                };

                // Escape backticks in the cypher text for template literal
                let escaped = cypher_text.replace('\\', "\\\\").replace('`', "\\`");

                Ok(format!(
                    "const {} = {{ __type: \"cql\", query: `{}`, params: {} }}",
                    name_text, escaped, params_obj
                ))
            }
            Expr::Error { .. } => {
                Err("parser error expression reached JS subset emitter".to_string())
            }
            Expr::Include { kind, expr, .. } => {
                self.uses_include_stub = true;
                let path = self.emit_expr(*expr)?;
                let kind_name = match kind {
                    php_rs::parser::ast::IncludeKind::Include => "include",
                    php_rs::parser::ast::IncludeKind::IncludeOnce => "include_once",
                    php_rs::parser::ast::IncludeKind::Require => "require",
                    php_rs::parser::ast::IncludeKind::RequireOnce => "require_once",
                };
                Ok(format!(
                    "__phpx_include({}, {})",
                    path,
                    json_string(kind_name)
                ))
            }
            Expr::MagicConst { kind, .. } => {
                let lowered = match kind {
                    php_rs::parser::ast::MagicConstKind::Line => "0".to_string(),
                    php_rs::parser::ast::MagicConstKind::Dir
                    | php_rs::parser::ast::MagicConstKind::File
                    | php_rs::parser::ast::MagicConstKind::Function
                    | php_rs::parser::ast::MagicConstKind::Class
                    | php_rs::parser::ast::MagicConstKind::Trait
                    | php_rs::parser::ast::MagicConstKind::Method
                    | php_rs::parser::ast::MagicConstKind::Namespace
                    | php_rs::parser::ast::MagicConstKind::Property => json_string(""),
                };
                Ok(lowered)
            }
            Expr::InterpolatedString { parts, .. } => {
                let mut pieces = Vec::new();
                for part in *parts {
                    pieces.push(self.emit_expr(*part)?);
                }
                Ok(format!("({})", pieces.join(" + ")))
            }
            Expr::Ternary {
                condition,
                if_true,
                if_false,
                ..
            } => {
                let cond = self.emit_expr(*condition)?;
                let when_true = if let Some(value) = if_true {
                    self.emit_expr(*value)?
                } else {
                    cond.clone()
                };
                let when_false = self.emit_expr(*if_false)?;
                Ok(format!("({} ? {} : {})", cond, when_true, when_false))
            }
            Expr::Match {
                condition, arms, ..
            } => self.emit_match_expr(*condition, arms),
            other => Err(format!(
                "unsupported expression in subset emitter: {:?}",
                other
            )),
        }
    }
}
