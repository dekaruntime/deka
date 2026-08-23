use super::*;

/// Precedence levels for JavaScript operators. Higher numeric values bind
/// more tightly. Used by `emit_expr_with_prec` to decide when a binary
/// expression must be parenthesized to preserve the source AST grouping.
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
        BinaryOp::Concat => (Prec::Add, Assoc::Left, "+"),
        // `instanceof` is handled separately in the caller before this mapping.
        _ => (Prec::Min, Assoc::Left, "?"),
    }
}

/// Heuristic to decide whether raw JavaScript inside an `unsafe { ... }` block
/// should be treated as a statement block or a single expression.
///
/// A statement block is one that contains a semicolon, curly braces, or starts
/// with a common JS statement keyword. Everything else is treated as an
/// expression and wrapped with `return deka.Result.Ok((expr));`.
fn raw_js_looks_like_statements(raw: &str) -> bool {
    if raw.contains(';') {
        return true;
    }
    let head = raw
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| c == '(' || c == '{' || c == '[');
    matches!(
        head,
        "const"
            | "let"
            | "var"
            | "function"
            | "class"
            | "if"
            | "for"
            | "while"
            | "do"
            | "try"
            | "switch"
            | "return"
            | "throw"
            | "break"
            | "continue"
            | "with"
            | "debugger"
            | "import"
            | "export"
            | "async"
    )
}

impl<'a> JsSubsetEmitter<'a> {
    pub(super) fn emit_expr(&mut self, expr: ExprId<'_>) -> Result<String, String> {
        self.emit_expr_with_prec(expr, Prec::Min)
    }

    fn emit_expr_with_prec(&mut self, expr: ExprId<'_>, min_prec: Prec) -> Result<String, String> {
        if let Expr::Binary { left, op, right, .. } = expr {
            // `instanceof` against a struct type is a special runtime helper,
            // not a raw JS `instanceof` expression.
            if matches!(op, BinaryOp::Instanceof) {
                if let Expr::Variable { name, .. } = right {
                    let ident = self.span_name(*name);
                    if self.struct_names.contains(&ident) {
                        let lhs = self.emit_expr_with_prec(*left, Prec::Min)?;
                        self.uses_deka_struct_helpers = true;
                        return Ok(format!("deka.isStruct({}, {})", lhs, ident));
                    }
                }
            }
            // Pipeline operator is also a special form. In DekaScript it
            // desugars to a normal call (`a |> f(b)` => `f(a, b)`); in PHPX
            // we keep the IIFE wrapper so existing code keeps working.
            if matches!(op, BinaryOp::Pipe) {
                return self.emit_ds_pipe(*left, *right);
            }

            let (op_prec, assoc, js_op) = binary_op_info(op);
            if op_prec < min_prec {
                let inner = self.emit_expr_with_prec(expr, Prec::Min)?;
                return Ok(format!("({})", inner));
            }
            let (left_min, right_min) = match assoc {
                Assoc::Left => (op_prec, op_prec.next()),
                Assoc::Right => (op_prec.next(), op_prec),
                Assoc::NonAssoc => (op_prec.next(), op_prec.next()),
            };
            let lhs = self.emit_expr_with_prec(*left, left_min)?;
            let rhs = self.emit_expr_with_prec(*right, right_min)?;
            return Ok(format!("{} {} {}", lhs, js_op, rhs));
        }
        self.emit_expr_inner(expr)
    }

    fn emit_ds_pipe(&mut self, left: ExprId<'_>, right: ExprId<'_>) -> Result<String, String> {
        let lhs = self.emit_expr_with_prec(left, Prec::Min)?;
        match right {
            Expr::Call { func, args, .. } => {
                let callee = self.emit_expr_with_prec(*func, Prec::Min)?;
                let mut all_args = vec![lhs];
                for arg in *args {
                    all_args.push(self.emit_expr_with_prec(arg.value, Prec::Min)?);
                }
                Ok(format!("{}({})", callee, all_args.join(", ")))
            }
            Expr::Variable { name, .. } => {
                let ident = self.span_name(*name);
                Ok(format!("{}({})", ident, lhs))
            }
            _ => {
                let rhs = self.emit_expr_with_prec(right, Prec::Min)?;
                Ok(format!("{}({})", rhs, lhs))
            }
        }
    }

    fn emit_expr_inner(&mut self, expr: ExprId<'_>) -> Result<String, String> {
        match expr {
            Expr::Variable { name, .. } => {
                let ident = self.span_name(*name);
                if ident == "this" {
                    return Ok("this".to_string());
                }
                // RFD 19: `self` inside an impl-block method body binds to
                // the JS `this` a regular (non-arrow) function already gets
                // for free when called as `receiver.method()`. Mirrors the
                // existing `this` passthrough immediately above rather than
                // inventing a second mechanism.
                if ident == "self" {
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
                    // Emit undeclared identifiers as bare names. They resolve
                    // through normal JavaScript global lookup, which keeps the
                    // RAW tab output readable. The prelude helper scan still
                    // catches references to PHPX builtins by also matching bare
                    // whole-word identifiers.
                    Ok(ident)
                }
            }
            Expr::Integer { value, .. }
            | Expr::Float { value, .. }
            | Expr::BigInt { value, .. } => {
                Ok(String::from_utf8_lossy(value).to_string())
            }
            Expr::Boolean { value, .. } => Ok(if *value { "true" } else { "false" }.to_string()),
            Expr::Null { .. } => Ok("null".to_string()),
            Expr::String { value, .. } => Ok(self.encode_php_string_literal(value)),
            Expr::Unary { op, expr, .. } => {
                let value = self.emit_expr_with_prec(*expr, Prec::Unary)?;
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
                Ok(format!("{}{}", js_op, value))
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
                    Expr::Variable { name, .. } if self.property_fetch_is_dynamic(*name) => {
                        // Dynamic property access ($obj->$k or $obj->{$k}): read
                        // the variable's VALUE and use it as a computed key.
                        // See property_fetch_is_dynamic for why Expr::Variable
                        // is ambiguous between bareword and dynamic property names.
                        let key = self.span_name(*name);
                        Ok(format!("{}[{}]", target_js, key))
                    }
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
                    Expr::Variable { name, .. } if self.property_fetch_is_dynamic(*name) => {
                        let key = self.span_name(*name);
                        Ok(format!("({})?.[{}]", target_js, key))
                    }
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
                    // DekaScript spread element: `{ ...expr }`
                    if let Expr::Spread { expr, .. } = item.value {
                        let spread_expr = self.emit_expr(expr)?;
                        entries.push(format!("...{}", spread_expr));
                        continue;
                    }
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
                let mut seen = HashSet::new();
                let mut entries = Vec::new();
                for field in *fields {
                    let key = self
                        .token_text(field.name)
                        .trim_start_matches('$')
                        .to_string();
                    seen.insert(key.clone());
                    let value = self.emit_expr(field.value)?;
                    entries.push(format!("{}: {}", json_string(&key), value));
                }
                if self.meta.is_ds {
                    self.uses_deka_struct_helpers = true;
                    if let Some(meta_fields) = self.struct_fields.get(&struct_name).cloned() {
                        for field in meta_fields {
                            if seen.contains(&field.name) {
                                continue;
                            }
                            if field.empty_embed {
                                entries.push(format!(
                                    "{}: {}({{}})",
                                    json_string(&field.name),
                                    field.name
                                ));
                            } else if field.optional {
                                entries.push(format!(
                                    "{}: Option.None",
                                    json_string(&field.name)
                                ));
                            }
                        }
                    }
                    Ok(format!("{}({{{}}})", struct_name, entries.join(", ")))
                } else {
                    let mut tagged_entries = Vec::new();
                    tagged_entries.push(format!(
                        "{}: {}",
                        json_string("__struct"),
                        json_string(&struct_name)
                    ));
                    tagged_entries.extend(entries);
                    Ok(format!(
                        "(() => {{ const __obj = {{{}}}; const __m = globalThis.__phpxStructMethods ? globalThis.__phpxStructMethods[{}] : null; if (__m) Object.assign(__obj, __m); return __obj; }})()",
                        tagged_entries.join(", "),
                        json_string(&struct_name)
                    ))
                }
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
                Ok(format!("{}++", target))
            }
            Expr::PostDec { var, .. } => {
                let target = self.emit_assignable_expr(*var)?;
                Ok(format!("{}--", target))
            }
            Expr::Empty { expr, .. } => {
                let value = self.emit_expr_with_prec(*expr, Prec::Unary)?;
                Ok(format!("!{}", value))
            }
            Expr::Print { expr, .. } => {
                let value = self.emit_expr(*expr)?;
                Ok(format!(
                    "(globalThis.__dekaPrint ? globalThis.__dekaPrint({}) : console.log({}), undefined)",
                    value, value
                ))
            }
            Expr::Await { expr, .. } => {
                let value = self.emit_expr_with_prec(*expr, Prec::Unary)?;
                Ok(format!("await {}", value))
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
                    pieces.push(self.emit_expr_with_prec(*part, Prec::Add)?);
                }
                Ok(format!("({})", pieces.join(" + ")))
            }
            Expr::Ternary {
                condition,
                if_true,
                if_false,
                ..
            } => {
                let cond = self.emit_expr_with_prec(*condition, Prec::Ternary)?;
                let when_true = if let Some(value) = if_true {
                    self.emit_expr_with_prec(*value, Prec::Ternary)?
                } else {
                    cond.clone()
                };
                let when_false = self.emit_expr_with_prec(*if_false, Prec::Ternary)?;
                Ok(format!("({} ? {} : {})", cond, when_true, when_false))
            }
            Expr::Match {
                condition, arms, ..
            } => self.emit_match_expr(*condition, arms),
            Expr::Bridge {
                kind, action, args, ..
            } => {
                let args_js = self.emit_call_args(args)?;
                let kind_js = String::from_utf8_lossy(kind).replace('"', "");
                let action_js = String::from_utf8_lossy(action).replace('"', "");
                Ok(format!(
                    "(function(){{var __r=__deka_host(\"{kind_js}\", \"{action_js}\", [{args_js}]);return __r&&__r.ok===true?Ok(__r.data):Err((__r&&__r.error)||\"host call failed\");}})()"
                ))
            }
            Expr::Unsafe { raw, .. } => {
                let raw_str = String::from_utf8_lossy(raw);
                let trimmed = raw_str.trim();
                if trimmed.is_empty() {
                    return Ok("(function(){try{return Ok(undefined);}catch(err){return Err(err);}})()".to_string());
                }

                // Decide whether the raw JS is a single expression or a statement
                // block. Expressions are emitted as `return deka.Result.Ok(expr);`,
                // while statement blocks are executed as the body of an inner IIFE
                // so that any `return` inside them returns from that inner function
                // and the completion value is wrapped in Ok.
                let is_statement_block = raw_js_looks_like_statements(trimmed);

                const UNSAFE_GLOBALS: &[&str] = &[
                    "fetch", "JSON", "URL", "URLSearchParams", "TextEncoder", "TextDecoder",
                    "Blob", "FormData", "Headers", "Request", "Response", "WebSocket", "crypto",
                    "atob", "btoa", "structuredClone", "queueMicrotask", "setTimeout",
                    "setInterval", "clearTimeout", "clearInterval",
                ];
                let restore_globals = if UNSAFE_GLOBALS.is_empty() {
                    String::new()
                } else {
                    format!(
                        "const {{{}}}=unsafe;",
                        UNSAFE_GLOBALS.join(",")
                    )
                };
                // RFD 27: unsafe is JS-mode, not a host back door. Shadow the
                // dispatcher names and hide them on a proxied globalThis.
                let hide_host = "const __g=globalThis;const Deno=void 0,__bridge=void 0,__bridge_async=void 0,__deka_wasm_call=void 0,__deka_wasm_call_async=void 0,__deka_host=void 0;const globalThis=new Proxy(__g,{get(t,p){if(p==='Deno'||p==='__bridge'||p==='__bridge_async'||p==='__deka_wasm_call'||p==='__deka_wasm_call_async'||p==='__deka_host')return void 0;return Reflect.get(t,p);}});";

                let inner = if is_statement_block {
                    format!(
                        "(function(){{{hide}{restore}{raw}}})()",
                        hide = hide_host,
                        restore = restore_globals,
                        raw = raw_str
                    )
                } else {
                    format!(
                        "(function(){{{hide}{restore}return ({raw});}})()",
                        hide = hide_host,
                        restore = restore_globals,
                        raw = raw_str
                    )
                };

                Ok(format!(
                    "(function(){{try{{return Ok({inner});}}catch(err){{return Err(err);}}}})()",
                ))
            }
            other => Err(format!(
                "unsupported expression in subset emitter: {:?}",
                other
            )),
        }
    }
}
