use super::*;

pub(crate) struct LineIndex {
    pub(crate) line_starts: Vec<usize>,
}

impl LineIndex {
    pub(crate) fn new(source: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (idx, byte) in source.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        Self { line_starts }
    }

    pub(crate) fn offset_to_position(&self, offset: usize) -> Position {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line_start = self.line_starts.get(line).copied().unwrap_or(0);
        Position {
            line: line as u32,
            character: offset.saturating_sub(line_start) as u32,
        }
    }

    pub(crate) fn position_to_offset(&self, position: Position) -> Option<usize> {
        let line = position.line as usize;
        let line_start = *self.line_starts.get(line)?;
        Some(line_start + position.character as usize)
    }
}

pub(crate) fn with_program<F, R>(source: &str, file_path: &str, f: F) -> Option<R>
where
    F: FnOnce(&Program, &[u8]) -> R,
{
    let arena = Bump::new();
    let result = compile_phpx(source, file_path, &arena);
    let program = result.ast?;
    Some(f(&program, source.as_bytes()))
}

#[derive(Default)]
pub(crate) struct SymbolIndex {
    pub(crate) functions: Vec<FunctionInfo>,
    pub(crate) structs: Vec<StructInfo>,
    pub(crate) interfaces: Vec<InterfaceInfo>,
    pub(crate) enums: Vec<EnumInfo>,
    pub(crate) type_aliases: Vec<TypeAliasInfo>,
    pub(crate) globals: Vec<VarInfo>,
    pub(crate) consts: Vec<ConstInfo>,
}

#[derive(Clone)]
pub(crate) struct FunctionInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
    pub(crate) signature: String,
    pub(crate) props_type: Option<String>,
    pub(crate) vars: Vec<VarInfo>,
    pub(crate) scope_span: Span,
}

#[derive(Clone)]
pub(crate) struct VarInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
    pub(crate) ty: Option<String>,
}

#[derive(Clone)]
pub(crate) struct StructInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
    pub(crate) fields: Vec<FieldInfo>,
}

#[derive(Clone)]
pub(crate) struct InterfaceInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
    pub(crate) fields: Vec<FieldInfo>,
}

#[derive(Clone)]
pub(crate) struct FieldInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
    pub(crate) ty: Option<String>,
}

#[derive(Clone)]
pub(crate) struct EnumInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
    pub(crate) cases: Vec<EnumCaseInfo>,
}

#[derive(Clone)]
pub(crate) struct EnumCaseInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
}

#[derive(Clone)]
pub(crate) struct TypeAliasInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
    pub(crate) ty: String,
}

#[derive(Clone)]
pub(crate) struct ConstInfo {
    pub(crate) name: String,
    pub(crate) span: Span,
}

impl SymbolIndex {
    pub(crate) fn hover_at(&self, offset: usize) -> Option<String> {
        for func in &self.functions {
            if span_contains(func.span, offset) {
                return Some(format!("```php\n{}\n```", func.signature));
            }
            for var in &func.vars {
                if span_contains(var.span, offset) {
                    let ty = var.ty.clone().unwrap_or_else(|| "unknown".to_string());
                    return Some(format!("```php\n{}: {}\n```", var.name, ty));
                }
            }
        }

        for strukt in &self.structs {
            if span_contains(strukt.span, offset) {
                let mut out = format!("```php\nstruct {}\n", strukt.name);
                for field in &strukt.fields {
                    let ty = field.ty.clone().unwrap_or_else(|| "mixed".to_string());
                    out.push_str(&format!(
                        "  ${}: {}\n",
                        field.name.trim_start_matches('$'),
                        ty
                    ));
                }
                out.push_str("```");
                return Some(out);
            }
            for field in &strukt.fields {
                if span_contains(field.span, offset) {
                    let ty = field.ty.clone().unwrap_or_else(|| "mixed".to_string());
                    return Some(format!(
                        "```php\n${}: {}\n```",
                        field.name.trim_start_matches('$'),
                        ty
                    ));
                }
            }
        }

        for iface in &self.interfaces {
            if span_contains(iface.span, offset) {
                let mut out = format!("```php\ninterface {} {{\n", iface.name);
                for field in &iface.fields {
                    let ty = field.ty.clone().unwrap_or_else(|| "mixed".to_string());
                    out.push_str(&format!(
                        "  ${}: {}\n",
                        field.name.trim_start_matches('$'),
                        ty
                    ));
                }
                out.push_str("}\n```");
                return Some(out);
            }
            for field in &iface.fields {
                if span_contains(field.span, offset) {
                    let ty = field.ty.clone().unwrap_or_else(|| "mixed".to_string());
                    return Some(format!(
                        "```php\n${}: {}\n```",
                        field.name.trim_start_matches('$'),
                        ty
                    ));
                }
            }
        }

        for en in &self.enums {
            if span_contains(en.span, offset) {
                let mut out = format!("```php\nenum {}\n", en.name);
                for case_info in &en.cases {
                    out.push_str(&format!("  case {}\n", case_info.name));
                }
                out.push_str("```");
                return Some(out);
            }
            for case_info in &en.cases {
                if span_contains(case_info.span, offset) {
                    return Some(format!("```php\n{}::{}\n```", en.name, case_info.name));
                }
            }
        }

        for alias in &self.type_aliases {
            if span_contains(alias.span, offset) {
                return Some(format!("```php\ntype {} = {}\n```", alias.name, alias.ty));
            }
        }

        for konst in &self.consts {
            if span_contains(konst.span, offset) {
                return Some(format!("```php\nconst {}\n```", konst.name));
            }
        }

        None
    }

    pub(crate) fn definition_at(
        &self,
        offset: usize,
        uri: &Url,
        line_index: &LineIndex,
        source: &[u8],
    ) -> Option<Location> {
        let word = word_at_offset(source, offset);
        let Some(word) = word else {
            return None;
        };

        if word.starts_with('$') {
            for func in &self.functions {
                if span_contains(func.scope_span, offset) {
                    let mut best: Option<&VarInfo> = None;
                    for var in &func.vars {
                        if var.name == word && var.span.start <= offset {
                            if best.is_none() || var.span.start > best.unwrap().span.start {
                                best = Some(var);
                            }
                        }
                    }
                    if let Some(var) = best {
                        return Some(Location {
                            uri: uri.clone(),
                            range: span_to_range(var.span, line_index),
                        });
                    }
                    return None;
                }
            }
            let mut best: Option<&VarInfo> = None;
            for var in &self.globals {
                if var.name == word && var.span.start <= offset {
                    if best.is_none() || var.span.start > best.unwrap().span.start {
                        best = Some(var);
                    }
                }
            }
            if let Some(var) = best {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(var.span, line_index),
                });
            }
        }

        for func in &self.functions {
            if func.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(func.span, line_index),
                });
            }
        }
        for strukt in &self.structs {
            if strukt.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(strukt.span, line_index),
                });
            }
        }
        for iface in &self.interfaces {
            if iface.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(iface.span, line_index),
                });
            }
        }
        for en in &self.enums {
            if en.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(en.span, line_index),
                });
            }
        }
        for alias in &self.type_aliases {
            if alias.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(alias.span, line_index),
                });
            }
        }
        for konst in &self.consts {
            if konst.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(konst.span, line_index),
                });
            }
        }
        None
    }

    #[allow(deprecated)]
    pub(crate) fn document_symbols(&self, line_index: &LineIndex) -> Vec<DocumentSymbol> {
        let mut symbols = Vec::new();
        for func in &self.functions {
            symbols.push(DocumentSymbol {
                name: func.name.clone(),
                detail: Some(func.signature.clone()),
                kind: SymbolKind::FUNCTION,
                range: span_to_range(func.scope_span, line_index),
                selection_range: span_to_range(func.span, line_index),
                children: None,
                deprecated: None,
                tags: None,
            });
        }
        for strukt in &self.structs {
            let fields = strukt
                .fields
                .iter()
                .map(|field| DocumentSymbol {
                    name: field.name.trim_start_matches('$').to_string(),
                    detail: field.ty.clone(),
                    kind: SymbolKind::FIELD,
                    range: span_to_range(field.span, line_index),
                    selection_range: span_to_range(field.span, line_index),
                    children: None,
                    deprecated: None,
                    tags: None,
                })
                .collect();
            symbols.push(DocumentSymbol {
                name: strukt.name.clone(),
                detail: Some("struct".to_string()),
                kind: SymbolKind::STRUCT,
                range: span_to_range(strukt.span, line_index),
                selection_range: span_to_range(strukt.span, line_index),
                children: Some(fields),
                deprecated: None,
                tags: None,
            });
        }
        for iface in &self.interfaces {
            let fields = iface
                .fields
                .iter()
                .map(|field| DocumentSymbol {
                    name: field.name.trim_start_matches('$').to_string(),
                    detail: field.ty.clone(),
                    kind: SymbolKind::FIELD,
                    range: span_to_range(field.span, line_index),
                    selection_range: span_to_range(field.span, line_index),
                    children: None,
                    deprecated: None,
                    tags: None,
                })
                .collect();
            symbols.push(DocumentSymbol {
                name: iface.name.clone(),
                detail: Some("interface".to_string()),
                kind: SymbolKind::INTERFACE,
                range: span_to_range(iface.span, line_index),
                selection_range: span_to_range(iface.span, line_index),
                children: Some(fields),
                deprecated: None,
                tags: None,
            });
        }
        for en in &self.enums {
            let cases = en
                .cases
                .iter()
                .map(|case_info| DocumentSymbol {
                    name: case_info.name.clone(),
                    detail: Some("case".to_string()),
                    kind: SymbolKind::ENUM_MEMBER,
                    range: span_to_range(case_info.span, line_index),
                    selection_range: span_to_range(case_info.span, line_index),
                    children: None,
                    deprecated: None,
                    tags: None,
                })
                .collect();
            symbols.push(DocumentSymbol {
                name: en.name.clone(),
                detail: Some("enum".to_string()),
                kind: SymbolKind::ENUM,
                range: span_to_range(en.span, line_index),
                selection_range: span_to_range(en.span, line_index),
                children: Some(cases),
                deprecated: None,
                tags: None,
            });
        }
        for alias in &self.type_aliases {
            symbols.push(DocumentSymbol {
                name: alias.name.clone(),
                detail: Some(alias.ty.clone()),
                kind: SymbolKind::TYPE_PARAMETER,
                range: span_to_range(alias.span, line_index),
                selection_range: span_to_range(alias.span, line_index),
                children: None,
                deprecated: None,
                tags: None,
            });
        }
        for konst in &self.consts {
            symbols.push(DocumentSymbol {
                name: konst.name.clone(),
                detail: Some("const".to_string()),
                kind: SymbolKind::CONSTANT,
                range: span_to_range(konst.span, line_index),
                selection_range: span_to_range(konst.span, line_index),
                children: None,
                deprecated: None,
                tags: None,
            });
        }
        symbols
    }

    pub(crate) fn var_type_at(&self, offset: usize, name: &str) -> Option<String> {
        for func in &self.functions {
            if span_contains(func.scope_span, offset) {
                let mut best: Option<&VarInfo> = None;
                for var in &func.vars {
                    if var.name == name && var.span.start <= offset {
                        if best.is_none() || var.span.start > best.unwrap().span.start {
                            best = Some(var);
                        }
                    }
                }
                return best.and_then(|var| var.ty.clone());
            }
        }
        let mut best: Option<&VarInfo> = None;
        for var in &self.globals {
            if var.name == name && var.span.start <= offset {
                if best.is_none() || var.span.start > best.unwrap().span.start {
                    best = Some(var);
                }
            }
        }
        best.and_then(|var| var.ty.clone())
    }

    pub(crate) fn fields_for_type(&self, ty: &str) -> Option<Vec<String>> {
        if let Some(strukt) = self.structs.iter().find(|s| s.name == ty) {
            return Some(
                strukt
                    .fields
                    .iter()
                    .map(|field| field.name.trim_start_matches('$').to_string())
                    .collect(),
            );
        }
        if let Some(iface) = self.interfaces.iter().find(|i| i.name == ty) {
            return Some(
                iface
                    .fields
                    .iter()
                    .map(|field| field.name.trim_start_matches('$').to_string())
                    .collect(),
            );
        }
        if let Some(alias) = self.type_aliases.iter().find(|alias| alias.name == ty) {
            return self.fields_for_type(alias.ty.trim());
        }
        if let Some(inner) = ty
            .strip_prefix("Option<")
            .and_then(|value| value.strip_suffix('>'))
        {
            return self.fields_for_type(inner.trim());
        }
        if let Some(inner) = ty
            .strip_prefix("Result<")
            .and_then(|value| value.strip_suffix('>'))
        {
            let first = inner.split(',').next().unwrap_or(inner).trim();
            return self.fields_for_type(first);
        }
        if let Some(inner) = ty
            .strip_prefix("Object<{")
            .and_then(|value| value.strip_suffix("}>"))
        {
            let mut fields = Vec::new();
            for part in inner.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let name = part
                    .split(':')
                    .next()
                    .unwrap_or(part)
                    .trim()
                    .trim_end_matches('?');
                if !name.is_empty() {
                    fields.push(name.to_string());
                }
            }
            return Some(fields);
        }
        None
    }

    pub(crate) fn component_prop_fields(
        &self,
        component: &str,
    ) -> Option<Vec<(String, Option<String>)>> {
        let func = self.functions.iter().find(|func| func.name == component)?;
        let ty = func.props_type.as_deref()?.trim();
        self.fields_for_type_with_types(ty)
    }

    pub(crate) fn fields_for_type_with_types(
        &self,
        ty: &str,
    ) -> Option<Vec<(String, Option<String>)>> {
        if let Some(strukt) = self.structs.iter().find(|s| s.name == ty) {
            return Some(
                strukt
                    .fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.trim_start_matches('$').to_string(),
                            field.ty.clone(),
                        )
                    })
                    .collect(),
            );
        }
        if let Some(iface) = self.interfaces.iter().find(|i| i.name == ty) {
            return Some(
                iface
                    .fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.trim_start_matches('$').to_string(),
                            field.ty.clone(),
                        )
                    })
                    .collect(),
            );
        }
        if let Some(alias) = self.type_aliases.iter().find(|alias| alias.name == ty) {
            return self.fields_for_type_with_types(alias.ty.trim());
        }
        if let Some(inner) = ty
            .strip_prefix("Option<")
            .and_then(|value| value.strip_suffix('>'))
        {
            return self.fields_for_type_with_types(inner.trim());
        }
        if let Some(inner) = ty
            .strip_prefix("Result<")
            .and_then(|value| value.strip_suffix('>'))
        {
            let first = inner.split(',').next().unwrap_or(inner).trim();
            return self.fields_for_type_with_types(first);
        }
        if let Some(inner) = ty
            .strip_prefix("Object<{")
            .and_then(|value| value.strip_suffix("}>"))
        {
            let mut fields = Vec::new();
            for part in inner.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let mut segments = part.split(':');
                let name = segments.next().unwrap_or(part).trim().trim_end_matches('?');
                let ty = segments.next().map(|t| t.trim().to_string());
                if !name.is_empty() {
                    fields.push((name.to_string(), ty));
                }
            }
            return Some(fields);
        }
        None
    }
}

pub(crate) fn build_index(program: &Program, source: &[u8]) -> SymbolIndex {
    let mut index = SymbolIndex::default();
    for stmt in program.statements.iter() {
        collect_stmt(stmt, source, &mut index);
    }
    index
}

pub(crate) fn collect_stmt(stmt: StmtId, source: &[u8], index: &mut SymbolIndex) {
    match stmt {
        Stmt::Expression { expr, .. } => {
            collect_vars_in_expr(expr, source, &mut index.globals);
        }
        Stmt::Return {
            expr: Some(expr), ..
        } => {
            collect_vars_in_expr(expr, source, &mut index.globals);
        }
        Stmt::Function {
            name,
            is_async,
            params,
            return_type,
            body,
            span,
            ..
        } => {
            let fn_name = token_text(source, name);
            let signature = format_function_signature(
                fn_name.as_str(),
                params,
                *return_type,
                source,
                *is_async,
            );
            let mut vars = Vec::new();
            for param in *params {
                let param_name = token_text(source, param.name);
                let ty = param.ty.map(|ty| format_type(ty, source));
                vars.push(VarInfo {
                    name: param_name,
                    span: param.name.span,
                    ty,
                });
            }
            collect_vars_in_block(body, source, &mut vars);
            index.functions.push(FunctionInfo {
                name: fn_name,
                span: name.span,
                signature,
                props_type: params
                    .first()
                    .and_then(|param| param.ty.map(|ty| format_type(ty, source))),
                vars,
                scope_span: *span,
            });
        }
        Stmt::Class {
            kind,
            name,
            members,
            span: _,
            ..
        } => {
            if *kind == ClassKind::Struct {
                let struct_name = token_text(source, name);
                let mut fields = Vec::new();
                for member in *members {
                    if let ClassMember::Property { ty, entries, .. } = member {
                        for entry in *entries {
                            let field_name = token_text(source, entry.name);
                            let field_ty = ty.map(|ty| format_type(ty, source));
                            fields.push(FieldInfo {
                                name: field_name,
                                span: entry.name.span,
                                ty: field_ty,
                            });
                        }
                    }
                }
                index.structs.push(StructInfo {
                    name: struct_name,
                    span: name.span,
                    fields,
                });
            }
        }
        Stmt::Interface {
            name,
            members,
            span: _,
            ..
        } => {
            let iface_name = token_text(source, name);
            let mut fields = Vec::new();
            for member in *members {
                if let ClassMember::Property { ty, entries, .. } = member {
                    for entry in *entries {
                        let field_name = token_text(source, entry.name);
                        let field_ty = ty.map(|ty| format_type(ty, source));
                        fields.push(FieldInfo {
                            name: field_name,
                            span: entry.name.span,
                            ty: field_ty,
                        });
                    }
                }
            }
            index.interfaces.push(InterfaceInfo {
                name: iface_name,
                span: name.span,
                fields,
            });
        }
        Stmt::Enum {
            name,
            members,
            span: _,
            ..
        } => {
            let enum_name = token_text(source, name);
            let mut cases = Vec::new();
            for member in *members {
                if let ClassMember::Case { name, .. } = member {
                    cases.push(EnumCaseInfo {
                        name: token_text(source, name),
                        span: name.span,
                    });
                }
            }
            index.enums.push(EnumInfo {
                name: enum_name,
                span: name.span,
                cases,
            });
        }
        Stmt::TypeAlias { name, ty, .. } => {
            let alias_name = token_text(source, name);
            let alias_ty = format_type(ty, source);
            index.type_aliases.push(TypeAliasInfo {
                name: alias_name,
                span: name.span,
                ty: alias_ty,
            });
        }
        Stmt::Const { consts, .. } => {
            for konst in *consts {
                let const_name = token_text(source, konst.name);
                index.consts.push(ConstInfo {
                    name: const_name,
                    span: konst.name.span,
                });
            }
        }
        Stmt::Block { statements, .. } => {
            for stmt in *statements {
                collect_stmt(stmt, source, index);
            }
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            for stmt in *then_block {
                collect_stmt(stmt, source, index);
            }
            if let Some(block) = else_block {
                for stmt in *block {
                    collect_stmt(stmt, source, index);
                }
            }
        }
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::For { body, .. }
        | Stmt::Foreach { body, .. } => {
            for stmt in *body {
                collect_stmt(stmt, source, index);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_vars_in_block(body: &[StmtId], source: &[u8], vars: &mut Vec<VarInfo>) {
    for stmt in body {
        match stmt {
            Stmt::Expression { expr, .. } => collect_vars_in_expr(expr, source, vars),
            Stmt::Return {
                expr: Some(expr), ..
            } => collect_vars_in_expr(expr, source, vars),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_vars_in_block(then_block, source, vars);
                if let Some(block) = else_block {
                    collect_vars_in_block(block, source, vars);
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Foreach { body, .. }
            | Stmt::Block {
                statements: body, ..
            } => {
                collect_vars_in_block(body, source, vars);
            }
            _ => {}
        }
    }
}

pub(crate) fn collect_vars_in_expr(expr: ExprId, source: &[u8], vars: &mut Vec<VarInfo>) {
    match expr {
        Expr::Assign { var, expr, .. }
        | Expr::AssignOp { var, expr, .. }
        | Expr::AssignRef { var, expr, .. } => {
            if let Some((name, span)) = variable_name(var, source) {
                let ty = infer_expr_type(expr, source);
                vars.push(VarInfo { name, span, ty });
            }
            collect_vars_in_expr(expr, source, vars);
        }
        Expr::Binary { left, right, .. } => {
            collect_vars_in_expr(left, source, vars);
            collect_vars_in_expr(right, source, vars);
        }
        Expr::Unary { expr, .. } => collect_vars_in_expr(expr, source, vars),
        Expr::Call { func, args, .. } => {
            collect_vars_in_expr(func, source, vars);
            for arg in *args {
                collect_vars_in_expr(arg.value, source, vars);
            }
        }
        Expr::Array { items, .. } => {
            for item in *items {
                collect_vars_in_expr(item.value, source, vars);
            }
        }
        Expr::ObjectLiteral { items, .. } => {
            for item in *items {
                collect_vars_in_expr(item.value, source, vars);
            }
        }
        Expr::StructLiteral { fields, .. } => {
            for field in *fields {
                collect_vars_in_expr(field.value, source, vars);
            }
        }
        Expr::JsxElement { children, .. } | Expr::JsxFragment { children, .. } => {
            for child in *children {
                if let php_rs::parser::ast::JsxChild::Expr(expr) = child {
                    collect_vars_in_expr(expr, source, vars);
                }
            }
        }
        Expr::ArrayDimFetch { array, dim, .. } => {
            collect_vars_in_expr(array, source, vars);
            if let Some(dim) = dim {
                collect_vars_in_expr(dim, source, vars);
            }
        }
        Expr::PropertyFetch {
            target, property, ..
        } => {
            collect_vars_in_expr(target, source, vars);
            collect_vars_in_expr(property, source, vars);
        }
        Expr::MethodCall {
            target,
            method,
            args,
            ..
        } => {
            collect_vars_in_expr(target, source, vars);
            collect_vars_in_expr(method, source, vars);
            for arg in *args {
                collect_vars_in_expr(arg.value, source, vars);
            }
        }
        Expr::StaticCall {
            class,
            method,
            args,
            ..
        } => {
            collect_vars_in_expr(class, source, vars);
            collect_vars_in_expr(method, source, vars);
            for arg in *args {
                collect_vars_in_expr(arg.value, source, vars);
            }
        }
        Expr::ClassConstFetch {
            class, constant, ..
        } => {
            collect_vars_in_expr(class, source, vars);
            collect_vars_in_expr(constant, source, vars);
        }
        Expr::New { class, args, .. } => {
            collect_vars_in_expr(class, source, vars);
            for arg in *args {
                collect_vars_in_expr(arg.value, source, vars);
            }
        }
        Expr::Ternary {
            condition,
            if_true,
            if_false,
            ..
        } => {
            collect_vars_in_expr(condition, source, vars);
            if let Some(expr) = if_true {
                collect_vars_in_expr(expr, source, vars);
            }
            collect_vars_in_expr(if_false, source, vars);
        }
        Expr::Include { expr, .. } => collect_vars_in_expr(expr, source, vars),
        Expr::DotAccess { target, .. } => collect_vars_in_expr(target, source, vars),
        Expr::PostInc { var, .. } | Expr::PostDec { var, .. } => {
            collect_vars_in_expr(var, source, vars)
        }
        Expr::InterpolatedString { parts, .. } | Expr::ShellExec { parts, .. } => {
            for part in *parts {
                collect_vars_in_expr(part, source, vars);
            }
        }
        _ => {}
    }
}

pub(crate) fn variable_name(expr: ExprId, source: &[u8]) -> Option<(String, Span)> {
    match expr {
        Expr::Variable { name, .. } => {
            let text = span_text(source, name);
            Some((text, *name))
        }
        _ => None,
    }
}

pub(crate) fn infer_expr_type(expr: ExprId, source: &[u8]) -> Option<String> {
    match expr {
        Expr::Integer { .. } => Some("int".to_string()),
        Expr::Float { .. } => Some("float".to_string()),
        Expr::Boolean { .. } => Some("bool".to_string()),
        Expr::String { .. } | Expr::InterpolatedString { .. } => Some("string".to_string()),
        Expr::Array { .. } => Some("array".to_string()),
        Expr::ObjectLiteral { items, .. } => {
            let mut fields = Vec::new();
            for item in *items {
                let key = match item.key {
                    ObjectKey::Ident(token) => token_text(source, token),
                    ObjectKey::String(token) => decode_string_key(&token_text(source, token)),
                };
                if !key.is_empty() {
                    fields.push(format!("{}: mixed", key));
                }
            }
            if fields.is_empty() {
                Some("Object".to_string())
            } else {
                Some(format!("Object<{{{}}}>", fields.join(", ")))
            }
        }
        Expr::StructLiteral { name, .. } => Some(name_text(source, name)),
        Expr::JsxElement { .. } | Expr::JsxFragment { .. } => Some("VNode".to_string()),
        Expr::Null { .. } => Some("null".to_string()),
        Expr::Binary {
            op: BinaryOp::Coalesce,
            left,
            right,
            ..
        } => {
            let left_ty = infer_expr_type(left, source);
            let right_ty = infer_expr_type(right, source);
            match (left_ty, right_ty) {
                (Some(left), Some(right)) if left == right => Some(left),
                (Some(left), Some(right)) => Some(format!("{} | {}", left, right)),
                (Some(left), None) => Some(left),
                (None, Some(right)) => Some(right),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

pub(crate) fn format_function_signature(
    name: &str,
    params: &[Param],
    return_type: Option<&Type>,
    source: &[u8],
    is_async: bool,
) -> String {
    let mut sig = if is_async {
        format!("async function {}(", name)
    } else {
        format!("function {}(", name)
    };
    let mut first = true;
    for param in params {
        if !first {
            sig.push_str(", ");
        }
        first = false;
        if let Some(ty) = param.ty {
            sig.push_str(&format_type(ty, source));
            sig.push(' ');
        }
        sig.push_str(&token_text(source, param.name));
    }
    sig.push(')');
    if let Some(ty) = return_type {
        sig.push_str(": ");
        sig.push_str(&format_type(ty, source));
    }
    sig
}

pub(crate) fn format_type(ty: &Type, source: &[u8]) -> String {
    match ty {
        Type::Simple(token) => token_text(source, token),
        Type::Name(name) => name_text(source, name),
        Type::Union(types) => types
            .iter()
            .map(|ty| format_type(ty, source))
            .collect::<Vec<_>>()
            .join(" | "),
        Type::Intersection(types) => types
            .iter()
            .map(|ty| format_type(ty, source))
            .collect::<Vec<_>>()
            .join(" & "),
        Type::Nullable(inner) => format!("?{}", format_type(inner, source)),
        Type::ObjectShape(fields) => {
            let mut out = String::from("Object<{");
            let mut first = true;
            for field in *fields {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                out.push_str(&token_text(source, field.name));
                if field.optional {
                    out.push('?');
                }
                out.push_str(": ");
                out.push_str(&format_type(field.ty, source));
            }
            out.push_str("}>");
            out
        }
        Type::Applied { base, args } => {
            let mut out = format_type(base, source);
            let rendered = args
                .iter()
                .map(|ty| format_type(ty, source))
                .collect::<Vec<_>>();
            out.push('<');
            out.push_str(&rendered.join(", "));
            out.push('>');
            out
        }
    }
}

pub(crate) fn name_text(source: &[u8], name: &Name) -> String {
    String::from_utf8_lossy(name.span.as_str(source)).to_string()
}

pub(crate) fn token_text(source: &[u8], token: &Token) -> String {
    String::from_utf8_lossy(token.text(source)).to_string()
}

pub(crate) fn span_text(source: &[u8], span: &Span) -> String {
    String::from_utf8_lossy(span.as_str(source)).to_string()
}

// Keep in sync with `decode_string_key` in
// runtime/crates/phpx_js/src/lib.rs and `parse_string_key` in
// runtime/crates/php-rs/src/phpx/typeck/check.rs. ObjectKey::String tokens
// retain their surrounding quotes and embedded escapes — we strip the matched
// quote pair and decode the few escapes the lexer accepts inside string keys.
pub(crate) fn decode_string_key(raw: &str) -> String {
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &raw[1..raw.len() - 1];
            return unescape_string_key(inner, first == b'"');
        }
    }
    raw.to_string()
}

pub(crate) fn unescape_string_key(value: &str, double_quoted: bool) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            '\'' if !double_quoted => out.push('\''),
            '"' if double_quoted => out.push('"'),
            '\\' => out.push('\\'),
            'n' if double_quoted => out.push('\n'),
            'r' if double_quoted => out.push('\r'),
            't' if double_quoted => out.push('\t'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

pub(crate) fn span_contains(span: Span, offset: usize) -> bool {
    span.start <= offset && offset < span.end
}

pub(crate) fn span_to_range(span: Span, line_index: &LineIndex) -> Range {
    Range {
        start: line_index.offset_to_position(span.start),
        end: line_index.offset_to_position(span.end),
    }
}

pub(crate) fn word_at_offset(source: &[u8], offset: usize) -> Option<String> {
    let bytes = source;
    if offset >= bytes.len() {
        return None;
    }
    let mut start = offset;
    let mut end = offset;
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_ident_char(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let word = &bytes[start..end];
    if word.iter().all(|b| b.is_ascii_whitespace()) {
        return None;
    }
    Some(String::from_utf8_lossy(word).to_string())
}

pub(crate) fn word_before_dot(source: &[u8], offset: usize) -> Option<String> {
    if offset == 0 || source.get(offset - 1) != Some(&b'.') {
        return None;
    }
    if offset < 2 || !is_ident_char(source[offset - 2]) {
        return None;
    }
    let mut start = offset - 2;
    while start > 0 && is_ident_char(source[start - 1]) {
        start -= 1;
    }
    let end = offset - 1;
    if start >= end {
        return None;
    }
    Some(String::from_utf8_lossy(&source[start..end]).to_string())
}

pub(crate) fn is_ident_char(byte: u8) -> bool {
    byte == b'$'
        || byte == b'_'
        || (byte >= b'0' && byte <= b'9')
        || (byte >= b'a' && byte <= b'z')
        || (byte >= b'A' && byte <= b'Z')
        || byte == b'\\'
}

pub(crate) fn hover_from_import(source: &str, offset: usize) -> Option<String> {
    let imports = parse_imports(source);
    for import in imports {
        if import.span.start <= offset && offset < import.span.end {
            let line = if import.imported == "default" {
                format!("import {} from '{}'", import.local, import.from)
            } else {
                format!("import {{ {} }} from '{}'", import.local, import.from)
            };
            return Some(format!("```php\n{}\n```", line));
        }
    }
    None
}

pub(crate) fn hover_for_annotation(source: &str, offset: usize) -> Option<String> {
    let name = annotation_name_at_offset(source, offset)?;
    let (_, detail) = annotation_catalog()
        .into_iter()
        .find(|(label, _)| *label == name.as_str())?;
    Some(format!("```php\n@{}\n```\n{}", name, detail))
}

pub(crate) fn annotation_name_at_offset(source: &str, offset: usize) -> Option<String> {
    let bytes = source.as_bytes();
    if offset > bytes.len() {
        return None;
    }
    let mut start = offset.min(bytes.len());
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = offset.min(bytes.len());
    while end < bytes.len() && is_ident_char(bytes[end]) {
        end += 1;
    }
    if start >= end {
        return None;
    }
    if start == 0 || bytes[start - 1] != b'@' {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[start..end]).to_string())
}
