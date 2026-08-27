//! Stateful JavaScript emitter for DekaScript compiler v2.
//!
//! Emits real runtime factories for structs (`deka.Struct`) and frozen case
//! objects for enums.  Receiver methods are registered on the struct factory
//! prototype so `p.greet()` works, including methods promoted from embedded
//! structs via the `deka.Struct` helper's embed map.

use std::collections::{HashMap, HashSet};

use deka_syntax::{BinOp, ExportDecl, Expr, ForInit, Pattern, Program, Stmt, Type};

use crate::util::{bin_op_str, escape_string, un_op_str, write_indent};

/// Emit JavaScript for a parsed and type-checked program.
pub fn emit_js(program: &Program, _source: &str) -> Result<String, String> {
    let mut emitter = Emitter::new(program);
    emitter.emit()
}

/// Emit JavaScript with imported module metadata available.
///
/// Imported structs and enums are seeded into the emitter so that struct
/// literals and enum constructors defined in other modules can be emitted
/// correctly in the current file.
pub fn emit_js_with_imports<'a>(
    program: &'a Program<'a>,
    _source: &str,
    imports: &HashMap<&str, &deka_syntax::ModuleExports<'a>>,
) -> Result<String, String> {
    let mut emitter = Emitter::new(program);
    emitter.seed_imports(imports);
    emitter.emit()
}

#[derive(Default, Clone)]
struct StructMeta {
    fields: HashSet<String>,
    embeds: Vec<String>,
    /// Optional fields: `None` means auto-fill with `Option.None`;
    /// `Some(expr)` means auto-fill with the pre-emitted default value.
    optional: HashMap<String, Option<String>>,
    empty_embeds: HashSet<String>,
}

#[derive(Default, Clone)]
struct EnumMeta {
    cases: Vec<String>,
    payload_cases: HashSet<String>,
}

#[derive(Clone)]
struct ReceiverMethod<'a> {
    name: String,
    params: Vec<String>,
    body: Vec<deka_syntax::Stmt<'a>>,
    is_async: bool,
}

struct Emitter<'a> {
    program: &'a Program<'a>,
    out: String,
    uses_struct: bool,
    uses_prelude_enums: bool,
    struct_order: Vec<String>,
    structs: HashMap<String, StructMeta>,
    enums: HashMap<String, EnumMeta>,
    receiver_methods: HashMap<String, Vec<ReceiverMethod<'a>>>,
}

impl<'a> Emitter<'a> {
    fn new(program: &'a Program<'a>) -> Self {
        let mut emitter = Self {
            program,
            out: String::new(),
            uses_struct: false,
            uses_prelude_enums: false,
            struct_order: Vec::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            receiver_methods: HashMap::new(),
        };
        emitter.prepass();
        emitter
    }

    fn emit(&mut self) -> Result<String, String> {
        self.emit_prelude()?;
        for (i, stmt) in self.program.statements.iter().enumerate() {
            if i > 0 {
                self.out.push('\n');
            }
            self.emit_stmt(stmt)?;
        }
        self.emit_method_registrations()?;
        Ok(std::mem::take(&mut self.out))
    }

    // ------------------------------------------------------------------
    // Pre-pass: collect struct/enum metadata and receiver methods.
    // ------------------------------------------------------------------
    fn prepass(&mut self) {
        for stmt in self.program.statements.iter() {
            match stmt {
                Stmt::Struct {
                    name,
                    fields,
                    embeds,
                    ..
                } => {
                    let mut meta = StructMeta::default();
                    for field in fields.iter() {
                        meta.fields.insert(field.name.to_string());
                        if field.default_value.is_some() || field.optional || is_optional_type(&field.ty) {
                            let default = field.default_value.as_ref().map(|v| {
                                // If a default value cannot be pre-emitted, fall back to null.
                                self.emit_expr_to_string(v).unwrap_or_else(|_| "null".to_string())
                            });
                            if default.is_none() {
                                // Omitted optional fields auto-fill to Option.None, so the
                                // prelude enum helpers are required even if the source never
                                // mentions Some/None explicitly.
                                self.uses_prelude_enums = true;
                            }
                            meta.optional.insert(field.name.to_string(), default);
                        }
                    }
                    for embed in embeds.iter() {
                        meta.embeds.push(embed.name.to_string());
                    }
                    self.structs.insert(name.to_string(), meta);
                    self.struct_order.push(name.to_string());
                }
                Stmt::Enum { name, cases, .. } => {
                    let mut meta = EnumMeta::default();
                    for case in cases.iter() {
                        meta.cases.push(case.name.to_string());
                        if case.payload.is_some() {
                            meta.payload_cases.insert(case.name.to_string());
                        }
                    }
                    self.enums.insert(name.to_string(), meta);
                }
                Stmt::ReceiverMethod {
                    receiver_type,
                    name,
                    params,
                    body,
                    is_async,
                    ..
                } => {
                    self.receiver_methods
                        .entry(receiver_type.to_string())
                        .or_default()
                        .push(ReceiverMethod {
                            name: name.to_string(),
                            params: params.iter().map(|p| p.name.to_string()).collect(),
                            body: body.to_vec(),
                            is_async: *is_async,
                        });
                }
                _ => {}
            }
        }
        self.compute_empty_embeds();
    }

    fn seed_imports(&mut self, imports: &HashMap<&str, &deka_syntax::ModuleExports<'a>>) {
        for exports in imports.values() {
            for (name, info) in exports.structs.iter() {
                if self.structs.contains_key(*name) {
                    continue;
                }
                let mut meta = StructMeta::default();
                for field in info.fields.iter() {
                    meta.fields.insert(field.name.to_string());
                    if field.default_value.is_some() || field.optional || is_optional_type(&field.ty) {
                        // Imported struct defaults are not pre-emitted here;
                        // omitting the field produces Option.None for Option-typed fields.
                        if field.default_value.is_none() {
                            self.uses_prelude_enums = true;
                        }
                        meta.optional.insert(field.name.to_string(), None);
                    }
                }
                for embed in info.embeds.iter() {
                    meta.embeds.push(embed.name.to_string());
                }
                self.structs.insert(name.to_string(), meta);
            }
            for (name, info) in exports.enums.iter() {
                if self.enums.contains_key(*name) {
                    continue;
                }
                let mut meta = EnumMeta::default();
                for case in info.cases.iter() {
                    meta.cases.push(case.name.to_string());
                    if case.payload.is_some() {
                        meta.payload_cases.insert(case.name.to_string());
                    }
                }
                self.enums.insert(name.to_string(), meta);
            }
        }
        self.compute_empty_embeds();
    }

    fn compute_empty_embeds(&mut self) {
        let names: Vec<String> = self.structs.keys().cloned().collect();
        let mut emptiness: HashMap<String, bool> = HashMap::new();
        for name in &names {
            emptiness.insert(name.clone(), self.is_empty_embed_struct(name));
        }
        for name in &names {
            let embeds = self.structs[name].embeds.clone();
            for embed in embeds {
                if *emptiness.get(&embed).unwrap_or(&false) {
                    self.structs
                        .get_mut(name)
                        .unwrap()
                        .empty_embeds
                        .insert(embed);
                }
            }
        }
    }

    fn is_empty_embed_struct(&self, name: &str) -> bool {
        let meta = match self.structs.get(name) {
            Some(m) => m,
            None => return false,
        };
        // A struct is an "empty embed" if it has no non-embed fields and all
        // of its embedded structs are also empty embeds.
        for field in &meta.fields {
            if !meta.embeds.contains(field) {
                return false;
            }
        }
        for embed in &meta.embeds {
            if !self.is_empty_embed_struct(embed) {
                return false;
            }
        }
        true
    }

    // ------------------------------------------------------------------
    // Prelude
    // ------------------------------------------------------------------
    fn emit_prelude(&mut self) -> Result<(), String> {
        // Determine which helpers are needed by scanning the AST.
        self.uses_struct = self.needs_struct_helper();
        self.uses_prelude_enums = self.uses_prelude_enums || self.needs_prelude_enums();

        if self.uses_struct {
            self.out.push_str("const __deka = {");
                        self.out.push_str(r###"Struct:(id,embeds)=>{function f(fields){const o=Object.create(f.prototype);Object.assign(o,fields);Object.defineProperty(o,'__deka_struct',{value:id,enumerable:false,writable:false,configurable:false});return o;}f.id=id;Object.defineProperty(f,'name',{value:id,configurable:true});f.prototype=Object.create(null);f.prototype.constructor=f;f.impl=(a,b)=>{if(typeof a==='string'){const k=a;f.prototype[k]=function(...x){return b(this,...x);};}else{for(const k in a)f.prototype[k]=a[k];}return f;};f.implMut=(a,b)=>{if(typeof a==='string'){const k=a;f.prototype[k]=function(...x){if(Object.isFrozen(this))throw new __deka.MutationError(`cannot call mutable method '${k}' on immutable ${id}`);return b(this,...x);};}else{for(const k in a){const fn=a[k];f.prototype[k]=function(...x){if(Object.isFrozen(this))throw new __deka.MutationError(`cannot call mutable method '${k}' on immutable ${id}`);return fn.apply(this,x);};}}return f;};if(embeds){for(const [embedName,embedFactory] of Object.entries(embeds)){for(const key of Object.keys(embedFactory.prototype)){f.prototype[key]=function(...args){return this[embedName][key](...args);};}}}return f;},"###);
            self.out.push_str("getStructId:(v)=>v?.__deka_struct,");
            self.out.push_str(
                "MutationError:class extends Error{constructor(m){super(m);this.name='MutationError';}}",
            );
            self.out.push_str("};\n");
            self.out.push_str("const deka = globalThis.deka = { ...globalThis.deka, ...__deka };\n");
        }

        if self.uses_prelude_enums {
            self.out.push_str("const Result = Object.freeze({\n");
            self.out.push_str("  Ok: (value) => Object.freeze({ __enum: \"Result\", __case: \"Ok\", name: \"Ok\", value }),\n");
            self.out.push_str("  Err: (error) => Object.freeze({ __enum: \"Result\", __case: \"Err\", name: \"Err\", error })\n");
            self.out.push_str("});\n");
            self.out.push_str("const Option = Object.freeze({\n");
            self.out.push_str("  Some: (value) => Object.freeze({ __enum: \"Option\", __case: \"Some\", name: \"Some\", value }),\n");
            self.out.push_str("  None: Object.freeze({ __enum: \"Option\", __case: \"None\", name: \"None\" })\n");
            self.out.push_str("});\n");
            self.out.push_str("const Ok = Result.Ok;\n");
            self.out.push_str("const Err = Result.Err;\n");
            self.out.push_str("const Some = Option.Some;\n");
            self.out.push_str("const None = Option.None;\n");
        }

        Ok(())
    }

    fn needs_struct_helper(&self) -> bool {
        !self.structs.is_empty() || !self.receiver_methods.is_empty()
    }

    fn needs_prelude_enums(&self) -> bool {
        self.program.statements.iter().any(|stmt| {
            let mut found = false;
            visit_stmt_exprs(stmt, &mut |expr| {
                if let Expr::EnumConstructor { enum_name, .. } = expr {
                    if *enum_name == "Option" || *enum_name == "Result" {
                        found = true;
                    }
                }
            });
            found
        })
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------
    fn emit_stmt(&mut self, stmt: &Stmt<'a>) -> Result<(), String> {
        match stmt {
            Stmt::Const { name, value, .. } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("const ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.emit_expr(value)?;
                self.out.push_str(";");
            }
            Stmt::Let { name, value, .. } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("let ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.emit_expr(value)?;
                self.out.push_str(";");
            }
            Stmt::Function {
                name,
                params,
                body,
                is_async,
                ..
            } => {
                write_indent(&mut self.out, 0);
                if *is_async {
                    self.out.push_str("async function ");
                } else {
                    self.out.push_str("function ");
                }
                self.out.push_str(name);
                self.out.push('(');
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.out.push_str(param.name);
                }
                self.out.push_str(") {\n");
                for stmt in body.iter() {
                    self.emit_stmt(stmt)?;
                    self.out.push('\n');
                }
                write_indent(&mut self.out, 0);
                self.out.push('}');
            }
            Stmt::Expr { expr, .. } => {
                write_indent(&mut self.out, 0);
                self.emit_expr(expr)?;
                self.out.push_str(";");
            }
            Stmt::Return { value, .. } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("return");
                if let Some(value) = value {
                    self.out.push(' ');
                    self.emit_expr(value)?;
                }
                self.out.push_str(";");
            }
            Stmt::Export { decl, .. } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("export ");
                match decl {
                    ExportDecl::Const { name, value, .. } => {
                        self.out.push_str("const ");
                        self.out.push_str(name);
                        self.out.push_str(" = ");
                        self.emit_expr(value)?;
                        self.out.push_str(";");
                    }
                    ExportDecl::Function {
                        name,
                        params,
                        body,
                        is_async,
                        ..
                    } => {
                        if *is_async {
                            self.out.push_str("async function ");
                        } else {
                            self.out.push_str("function ");
                        }
                        self.out.push_str(name);
                        self.out.push('(');
                        for (i, param) in params.iter().enumerate() {
                            if i > 0 {
                                self.out.push_str(", ");
                            }
                            self.out.push_str(param.name);
                        }
                        self.out.push_str(") {\n");
                        for stmt in body.iter() {
                            self.emit_stmt(stmt)?;
                            self.out.push('\n');
                        }
                        write_indent(&mut self.out, 0);
                        self.out.push('}');
                    }
                    ExportDecl::NamedGroup { names } => {
                        self.out.push_str("{ ");
                        for (i, name) in names.iter().enumerate() {
                            if i > 0 {
                                self.out.push_str(", ");
                            }
                            self.out.push_str(name.name);
                            if let Some(alias) = name.alias {
                                self.out.push_str(" as ");
                                self.out.push_str(alias);
                            }
                        }
                        self.out.push_str(" };");
                    }
                }
            }
            Stmt::Import { specifiers, source, .. } => {
                write_indent(&mut self.out, 0);
                if specifiers.is_empty() {
                    self.out.push_str("import \"");
                    self.out.push_str(source);
                    self.out.push_str("\";");
                } else {
                    self.out.push_str("import { ");
                    for (i, spec) in specifiers.iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(", ");
                        }
                        if spec.imported == spec.local {
                            self.out.push_str(spec.imported);
                        } else {
                            self.out.push_str(spec.imported);
                            self.out.push_str(" as ");
                            self.out.push_str(spec.local);
                        }
                    }
                    self.out.push_str(" } from \"");
                    self.out.push_str(source);
                    self.out.push_str("\";");
                }
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("if (");
                self.emit_expr(condition)?;
                self.out.push_str(") {\n");
                for stmt in then_body.iter() {
                    self.emit_stmt(stmt)?;
                    self.out.push('\n');
                }
                write_indent(&mut self.out, 0);
                self.out.push('}');
                if !else_body.is_empty() {
                    self.out.push_str(" else {\n");
                    for stmt in else_body.iter() {
                        self.emit_stmt(stmt)?;
                        self.out.push('\n');
                    }
                    write_indent(&mut self.out, 0);
                    self.out.push('}');
                }
            }
            Stmt::For {
                init,
                condition,
                step,
                body,
                ..
            } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("for (");
                if let Some(init) = init {
                    self.emit_for_init(init)?;
                }
                self.out.push_str("; ");
                if let Some(condition) = condition {
                    self.emit_expr(condition)?;
                }
                self.out.push_str("; ");
                if let Some(step) = step {
                    self.emit_expr(step)?;
                }
                self.out.push_str(") {\n");
                for stmt in body.iter() {
                    self.emit_stmt(stmt)?;
                    self.out.push('\n');
                }
                write_indent(&mut self.out, 0);
                self.out.push('}');
            }
            Stmt::Struct { name, embeds, .. } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("const ");
                self.out.push_str(name);
                self.out.push_str(" = deka.Struct(");
                self.out.push_str(&json_string(name));
                if !embeds.is_empty() {
                    self.out.push_str(", { ");
                    for (i, embed) in embeds.iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(", ");
                        }
                        self.out.push_str(embed.name);
                        self.out.push_str(": ");
                        self.out.push_str(embed.name);
                    }
                    self.out.push_str(" }");
                }
                self.out.push_str(");");
            }
            Stmt::Enum { name, cases, .. } => {
                self.emit_enum_object(name, cases)?;
            }
            Stmt::TypeAlias { .. } => {
                // Erased at runtime.
            }
            Stmt::Break { .. } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("break;");
            }
            Stmt::Continue { .. } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("continue;");
            }
            Stmt::ReceiverMethod { .. } => {
                // Collected in the pre-pass and emitted after all struct
                // factories have been declared.
            }
            Stmt::Empty { .. } => {
                // No output.
            }
        }
        Ok(())
    }

    fn emit_enum_object(
        &mut self,
        name: &str,
        cases: &[deka_syntax::EnumCase<'a>],
    ) -> Result<(), String> {
        write_indent(&mut self.out, 0);
        self.out.push_str("const ");
        self.out.push_str(name);
        self.out.push_str(" = Object.freeze({\n");
        for (i, case) in cases.iter().enumerate() {
            write_indent(&mut self.out, 2);
            self.out.push_str(&case.name);
            if let Some(payload_ty) = &case.payload {
                self.out.push_str("(value) => Object.freeze({ ");
                self.out.push_str("__enum: ");
                self.out.push_str(&json_string(name));
                self.out.push_str(", __case: ");
                self.out.push_str(&json_string(&case.name));
                self.out.push_str(", name: ");
                self.out.push_str(&json_string(&case.name));
                self.out.push_str(", value })");
                let _ = payload_ty; // type-only, no runtime effect
            } else {
                self.out.push_str(": Object.freeze({ ");
                self.out.push_str("__enum: ");
                self.out.push_str(&json_string(name));
                self.out.push_str(", __case: ");
                self.out.push_str(&json_string(&case.name));
                self.out.push_str(", name: ");
                self.out.push_str(&json_string(&case.name));
                self.out.push_str(" })");
            }
            if i + 1 < cases.len() {
                self.out.push(',');
            }
            self.out.push('\n');
        }
        write_indent(&mut self.out, 0);
        self.out.push_str("});");
        Ok(())
    }

    fn emit_method_registrations(&mut self) -> Result<(), String> {
        let order = self.struct_order.clone();
        for struct_name in order {
            if let Some(methods) = self.receiver_methods.get(&struct_name).cloned() {
                for method in methods {
                    write_indent(&mut self.out, 0);
                    self.out.push_str(&struct_name);
                    self.out.push_str(".impl(");
                    self.out.push_str(&json_string(&method.name));
                    self.out.push_str(", ");
                    if method.is_async {
                        self.out.push_str("async ");
                    }
                    self.out.push_str("function(");
                    for (i, param) in method.params.iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(", ");
                        }
                        self.out.push_str(param);
                    }
                    self.out.push_str(") {\n");
                    for stmt in method.body.iter() {
                        self.emit_stmt(stmt)?;
                        self.out.push('\n');
                    }
                    write_indent(&mut self.out, 0);
                    self.out.push_str("});\n");
                }
            }
        }
        Ok(())
    }

    fn emit_for_init(&mut self, init: &ForInit<'a>) -> Result<(), String> {
        match init {
            ForInit::Const { name, value } => {
                self.out.push_str("const ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.emit_expr(value)?;
            }
            ForInit::Let { name, value } => {
                self.out.push_str("let ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.emit_expr(value)?;
            }
            ForInit::Expr(expr) => {
                self.emit_expr(expr)?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------
    fn emit_expr_to_string(&mut self, expr: &Expr<'a>) -> Result<String, String> {
        let mut tmp = String::new();
        std::mem::swap(&mut self.out, &mut tmp);
        let res = self.emit_expr(expr);
        std::mem::swap(&mut self.out, &mut tmp);
        res?;
        Ok(tmp)
    }

    fn emit_expr(&mut self, expr: &Expr<'a>) -> Result<(), String> {
        match expr {
            Expr::Number { value, .. } => {
                if value.is_nan() {
                    self.out.push_str("NaN");
                } else if value.is_infinite() {
                    if value.is_sign_negative() {
                        self.out.push_str("-Infinity");
                    } else {
                        self.out.push_str("Infinity");
                    }
                } else if *value == 0.0 && value.is_sign_negative() {
                    self.out.push_str("-0");
                } else {
                    self.out.push_str(&format!("{}", value));
                }
            }
            Expr::BigInt { value, .. } => {
                self.out.push_str(value);
                self.out.push('n');
            }
            Expr::String { value, .. } => {
                self.out.push('"');
                self.out.push_str(&escape_string(value));
                self.out.push('"');
            }
            Expr::Boolean { value, .. } => {
                self.out.push_str(if *value { "true" } else { "false" });
            }
            Expr::None { .. } => {
                self.out.push_str("null");
            }
            Expr::Identifier { name, .. } => {
                self.out.push_str(name);
            }
            Expr::Binary { op, left, right, .. } => {
                if *op == BinOp::Pipe {
                    self.out.push('(');
                    self.emit_expr(right)?;
                    self.out.push_str(")(");
                    self.emit_expr(left)?;
                    self.out.push(')');
                } else {
                    self.emit_expr(left)?;
                    self.out.push(' ');
                    self.out.push_str(bin_op_str(*op));
                    self.out.push(' ');
                    self.emit_expr(right)?;
                }
            }
            Expr::Unary { op, operand, .. } => {
                self.out.push_str(un_op_str(*op));
                self.emit_expr(operand)?;
            }
            Expr::Call { callee, args, .. } => {
                self.emit_expr(callee)?;
                self.out.push('(');
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.emit_expr(arg)?;
                }
                self.out.push(')');
            }
            Expr::FieldAccess { object, field, .. } => {
                self.emit_expr(object)?;
                self.out.push('.');
                self.out.push_str(field);
            }
            Expr::StructLiteral { name, fields, .. } => {
                self.emit_struct_literal(name, fields)?;
            }
            Expr::IndexAccess { object, index, .. } => {
                self.emit_expr(object)?;
                self.out.push('[');
                self.emit_expr(index)?;
                self.out.push(']');
            }
            Expr::Paren { expr, .. } => {
                self.out.push('(');
                self.emit_expr(expr)?;
                self.out.push(')');
            }
            Expr::Array { elements, .. } => {
                self.out.push('[');
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.emit_expr(element)?;
                }
                self.out.push(']');
            }
            Expr::Object { fields, .. } => {
                self.out.push_str("{");
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    if field.key.is_empty() {
                        self.out.push_str("...");
                        self.emit_expr(&field.value)?;
                    } else {
                        self.out.push_str(field.key);
                        self.out.push_str(": ");
                        self.emit_expr(&field.value)?;
                    }
                }
                self.out.push_str("}");
            }
            Expr::Spread { expr, .. } => {
                self.out.push_str("...");
                self.emit_expr(expr)?;
            }
            Expr::Unsafe { source, .. } => {
                self.emit_unsafe(source)?;
            }
            Expr::EnumConstructor {
                enum_name,
                case_name,
                payload,
                ..
            } => {
                if *enum_name == "Option" || *enum_name == "Result" {
                    self.uses_prelude_enums = true;
                }
                self.out.push_str(enum_name);
                self.out.push('.');
                self.out.push_str(case_name);
                if let Some(payload) = payload {
                    self.out.push('(');
                    self.emit_expr(payload)?;
                    self.out.push(')');
                }
            }
            Expr::Match { scrutinee, arms, .. } => {
                self.emit_match(scrutinee, arms)?;
            }
            Expr::Await { expr, .. } => {
                self.out.push_str("await ");
                self.emit_expr(expr)?;
            }
            Expr::JsxElement { element, .. } => {
                self.emit_jsx_element(element)?;
            }
            Expr::JsxFragment { children, .. } => {
                self.emit_jsx_fragment(children)?;
            }
            Expr::JsxText { value, .. } => {
                self.out.push('"');
                self.out.push_str(&escape_string(value));
                self.out.push('"');
            }
            Expr::TemplateLiteral { parts, .. } => {
                self.out.push('`');
                for part in parts.iter() {
                    match part {
                        deka_syntax::TemplatePart::Text(text) => self.out.push_str(text),
                        deka_syntax::TemplatePart::Expr(expr) => {
                            self.out.push_str("${");
                            self.emit_expr(expr)?;
                            self.out.push('}');
                        }
                    }
                }
                self.out.push('`');
            }
            Expr::Function {
                params,
                body,
                is_async,
                ..
            } => {
                if *is_async {
                    self.out.push_str("async function(");
                } else {
                    self.out.push_str("function(");
                }
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.out.push_str(param.name);
                }
                self.out.push_str(") {\n");
                for stmt in body.iter() {
                    self.emit_stmt(stmt)?;
                    self.out.push('\n');
                }
                self.out.push('}');
            }
        }
        Ok(())
    }

    fn emit_struct_literal(
        &mut self,
        name: &str,
        fields: &[deka_syntax::StructLiteralField<'a>],
    ) -> Result<(), String> {
        let meta = self
            .structs
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown struct `{}` in emitter", name))?;

        self.uses_struct = true;
        let mut seen = HashSet::new();
        let mut entries = Vec::new();

        for field in fields.iter() {
            seen.insert(field.name.to_string());
            let mut value_buf = String::new();
            std::mem::swap(&mut self.out, &mut value_buf);
            self.emit_expr(&field.value)?;
            std::mem::swap(&mut self.out, &mut value_buf);
            let key = if is_js_identifier(field.name) {
                field.name.to_string()
            } else {
                json_string(field.name)
            };
            entries.push(format!("{}: {}", key, value_buf));
        }

        // Auto-fill empty embedded structs.
        for embed in &meta.embeds {
            if !seen.contains(embed) && meta.empty_embeds.contains(embed) {
                entries.push(format!("{}: {}({{}})", embed, embed));
            }
        }

        // Auto-fill omitted optional fields.
        for (opt, default) in &meta.optional {
            if !seen.contains(opt) {
                let value = match default {
                    Some(expr) => expr.clone(),
                    None => {
                        self.uses_prelude_enums = true;
                        "None".to_string()
                    }
                };
                entries.push(format!("{}: {}", opt, value));
            }
        }

        self.out.push_str(name);
        self.out.push_str("({ ");
        self.out.push_str(&entries.join(", "));
        self.out.push_str(" })");
        Ok(())
    }

    fn emit_unsafe(&mut self, source: &str) -> Result<(), String> {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            self.out.push_str("(function() { try { return { __case: \"Ok\", value: undefined }; } catch (err) { return { __case: \"Err\", value: err }; } })()");
            return Ok(());
        }

        let is_async = js_has_top_level_await(trimmed);
        let is_statement_block = raw_js_looks_like_statements(trimmed);

        let fn_kw = if is_async { "async function" } else { "function" };
        let inner = if is_statement_block {
            format!("({fn_kw}() {{ {trimmed} }})()")
        } else {
            format!("({fn_kw}() {{ return ({trimmed}); }})()")
        };

        let awaited = if is_async {
            format!("await {inner}")
        } else {
            inner
        };

        self.out.push('(');
        self.out.push_str(fn_kw);
        self.out.push_str("() { try { return { __case: \"Ok\", value: ");
        self.out.push_str(&awaited);
        self.out.push_str(" }; } catch (err) { return { __case: \"Err\", value: err }; } })()");

        Ok(())
    }

    fn emit_match(
        &mut self,
        scrutinee: &Expr<'a>,
        arms: &[deka_syntax::MatchArm<'a>],
    ) -> Result<(), String> {
        let scrutinee_var = "__deka_scrutinee";
        self.out.push_str("((");
        self.out.push_str(scrutinee_var);
        self.out.push_str(") => {\n");

        for (i, arm) in arms.iter().enumerate() {
            let is_last = i == arms.len() - 1;
            self.emit_match_arm(arm, scrutinee_var, is_last)?;
        }

        self.out.push_str("  throw new Error(\"non-exhaustive match\");\n");
        self.out.push_str("})(");
        self.emit_expr(scrutinee)?;
        self.out.push_str(")");
        Ok(())
    }

    fn emit_match_arm(
        &mut self,
        arm: &deka_syntax::MatchArm<'a>,
        scrutinee_var: &str,
        is_last: bool,
    ) -> Result<(), String> {
        let condition = self.match_condition(&arm.pattern, scrutinee_var);

        if condition == "true" && is_last {
            self.emit_pattern_bindings(&arm.pattern, scrutinee_var, 2)?;
            write_indent(&mut self.out, 2);
            self.out.push_str("return ");
            self.emit_expr(&arm.body)?;
            self.out.push_str(";\n");
            return Ok(());
        }

        write_indent(&mut self.out, 2);
        self.out.push_str("if (");
        self.out.push_str(&condition);
        self.out.push_str(") {\n");
        self.emit_pattern_bindings(&arm.pattern, scrutinee_var, 4)?;
        write_indent(&mut self.out, 4);
        self.out.push_str("return ");
        self.emit_expr(&arm.body)?;
        self.out.push_str(";\n");
        write_indent(&mut self.out, 2);
        self.out.push_str("}\n");
        Ok(())
    }

    fn match_condition(&self, pattern: &Pattern, scrutinee_var: &str) -> String {
        match pattern {
            Pattern::Wildcard { .. } => "true".to_string(),
            Pattern::Identifier { .. } => "true".to_string(),
            Pattern::Literal { expr, .. } => {
                let mut literal = String::new();
                // Literal patterns are always simple literals; reuse expr emission.
                let mut tmp = Emitter {
                    program: self.program,
                    out: literal,
                    uses_struct: false,
                    uses_prelude_enums: false,
                    struct_order: Vec::new(),
                    structs: HashMap::new(),
                    enums: HashMap::new(),
                    receiver_methods: HashMap::new(),
                };
                tmp.emit_expr(expr).expect("literal emission");
                literal = tmp.out;
                format!("{} === {}", scrutinee_var, literal)
            }
            Pattern::Constructor { name, .. } => {
                format!("{}.__case === \"{}\"", scrutinee_var, name)
            }
            Pattern::Struct { .. } | Pattern::Tuple { .. } => "false".to_string(),
        }
    }

    fn emit_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        scrutinee_var: &str,
        indent: usize,
    ) -> Result<(), String> {
        match pattern {
            Pattern::Wildcard { .. } => {}
            Pattern::Identifier { name, .. } => {
                write_indent(&mut self.out, indent);
                self.out.push_str("const ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.out.push_str(scrutinee_var);
                self.out.push_str(";\n");
            }
            Pattern::Literal { .. } => {}
            Pattern::Constructor { name, payload, .. } => {
                if let Some(payload) = payload {
                    let payload_access = if *name == "None" {
                        scrutinee_var.to_string()
                    } else {
                        format!("{}.value", scrutinee_var)
                    };
                    self.emit_pattern_bindings(payload, &payload_access, indent)?;
                }
            }
            Pattern::Struct { .. } | Pattern::Tuple { .. } => {}
        }
        Ok(())
    }

    fn emit_jsx_element(&mut self, element: &deka_syntax::JsxElement<'a>) -> Result<(), String> {
        let is_component = element
            .tag
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
        let tag_expr = if is_component {
            element.tag.to_string()
        } else {
            format!("\"{}\"", escape_string(element.tag))
        };

        let mut props = Vec::new();
        for attr in element.attributes.iter() {
            if attr.name.is_empty() {
                if let Some(value) = &attr.value {
                    let mut buf = String::new();
                    std::mem::swap(&mut self.out, &mut buf);
                    self.emit_expr(value)?;
                    std::mem::swap(&mut self.out, &mut buf);
                    props.push(format!("...{}", buf));
                }
            } else {
                let value = match &attr.value {
                    Some(v) => {
                        let mut buf = String::new();
                        std::mem::swap(&mut self.out, &mut buf);
                        self.emit_expr(v)?;
                        std::mem::swap(&mut self.out, &mut buf);
                        buf
                    }
                    None => "true".to_string(),
                };
                props.push(format!("\"{}\": {}", escape_string(attr.name), value));
            }
        }

        let mut child_values = Vec::new();
        for child in element.children.iter() {
            let mut buf = String::new();
            std::mem::swap(&mut self.out, &mut buf);
            self.emit_expr(child)?;
            std::mem::swap(&mut self.out, &mut buf);
            child_values.push(buf);
        }

        if !child_values.is_empty() {
            if child_values.len() == 1 {
                props.push(format!("\"children\": {}", child_values[0]));
            } else {
                props.push(format!("\"children\": [{}]", child_values.join(", ")));
            }
        }

        let fn_name = if child_values.len() > 1 { "jsxs" } else { "jsx" };
        self.out.push_str("deka.ui.");
        self.out.push_str(fn_name);
        self.out.push('(');
        self.out.push_str(&tag_expr);
        self.out.push_str(", {");
        self.out.push_str(&props.join(", "));
        self.out.push_str("})");
        Ok(())
    }

    fn emit_jsx_fragment(&mut self, children: &[Expr<'a>]) -> Result<(), String> {
        let mut child_values = Vec::new();
        for child in children.iter() {
            let mut buf = String::new();
            std::mem::swap(&mut self.out, &mut buf);
            self.emit_expr(child)?;
            std::mem::swap(&mut self.out, &mut buf);
            child_values.push(buf);
        }

        let fn_name = if child_values.len() > 1 { "jsxs" } else { "jsx" };
        self.out.push_str("deka.ui.");
        self.out.push_str(fn_name);
        self.out.push_str("(deka.ui.Fragment, {");
        if !child_values.is_empty() {
            if child_values.len() == 1 {
                self.out.push_str("\"children\": ");
                self.out.push_str(&child_values[0]);
            } else {
                self.out.push_str("\"children\": [");
                self.out.push_str(&child_values.join(", "));
                self.out.push(']');
            }
        }
        self.out.push_str("})");
        Ok(())
    }
}

fn is_optional_type(ty: &Type) -> bool {
    matches!(ty, Type::Option { .. })
}

fn json_string(s: &str) -> String {
    format!("\"{}\"", escape_string(s))
}

fn is_js_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_alphabetic() && first != '_' && first != '$' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

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

fn js_has_top_level_await(raw: &str) -> bool {
    raw.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word == "await")
}

fn visit_stmt_exprs(stmt: &Stmt, visitor: &mut dyn FnMut(&Expr)) {
    match stmt {
        Stmt::Const { value, .. }
        | Stmt::Let { value, .. }
        | Stmt::Expr { expr: value, .. }
        | Stmt::Return { value: Some(value), .. } => visit_expr(value, visitor),
        Stmt::Export { decl, .. } => match decl {
            ExportDecl::Const { value, .. } => visit_expr(value, visitor),
            ExportDecl::Function { body, .. } => {
                for s in body.iter() {
                    visit_stmt_exprs(s, visitor);
                }
            }
            _ => {}
        },
        Stmt::Function { body, .. }
        | Stmt::ReceiverMethod { body, .. }
        | Stmt::For { body, .. } => {
            for s in body.iter() {
                visit_stmt_exprs(s, visitor);
            }
        }
        Stmt::If { then_body, else_body, .. } => {
            for s in then_body.iter() {
                visit_stmt_exprs(s, visitor);
            }
            for s in else_body.iter() {
                visit_stmt_exprs(s, visitor);
            }
        }
        _ => {}
    }
}

fn visit_expr(expr: &Expr, visitor: &mut dyn FnMut(&Expr)) {
    visitor(expr);
    match expr {
        Expr::Binary { left, right, .. } => {
            visit_expr(left, visitor);
            visit_expr(right, visitor);
        }
        Expr::Unary { operand, .. } => visit_expr(operand, visitor),
        Expr::Call { callee, args, .. } => {
            visit_expr(callee, visitor);
            for a in args.iter() {
                visit_expr(a, visitor);
            }
        }
        Expr::FieldAccess { object, .. }
        | Expr::IndexAccess { object, .. }
        | Expr::Await { expr: object, .. }
        | Expr::Paren { expr: object, .. }
        | Expr::Spread { expr: object, .. } => visit_expr(object, visitor),
        Expr::StructLiteral { fields, .. } => {
            for f in fields.iter() {
                visit_expr(&f.value, visitor);
            }
        }
        Expr::EnumConstructor { payload, .. } => {
            if let Some(p) = payload {
                visit_expr(p, visitor);
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            visit_expr(scrutinee, visitor);
            for arm in arms.iter() {
                if let Some(guard) = &arm.guard {
                    visit_expr(guard, visitor);
                }
                visit_expr(&arm.body, visitor);
            }
        }
        Expr::Array { elements, .. } => {
            for e in elements.iter() {
                visit_expr(e, visitor);
            }
        }
        Expr::Object { fields, .. } => {
            for f in fields.iter() {
                visit_expr(&f.value, visitor);
            }
        }
        Expr::TemplateLiteral { parts, .. } => {
            for part in parts.iter() {
                if let deka_syntax::TemplatePart::Expr(e) = part {
                    visit_expr(e, visitor);
                }
            }
        }
        Expr::Function { body, .. } => {
            for s in body.iter() {
                visit_stmt_exprs(s, visitor);
            }
        }
        Expr::JsxElement { element, .. } => {
            for attr in element.attributes.iter() {
                if let Some(v) = &attr.value {
                    visit_expr(v, visitor);
                }
            }
            for child in element.children.iter() {
                visit_expr(child, visitor);
            }
        }
        Expr::JsxFragment { children, .. } => {
            for child in children.iter() {
                visit_expr(child, visitor);
            }
        }
        _ => {}
    }
}
