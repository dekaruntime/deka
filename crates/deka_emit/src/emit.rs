//! Stateful JavaScript emitter for DekaScript compiler v2.
//!
//! Emits real runtime factories for structs (`deka.Struct`) and frozen case
//! objects for enums.  Receiver methods are registered on the struct factory
//! prototype so `p.greet()` works, including methods promoted from embedded
//! structs via the `deka.Struct` helper's embed map.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use deka_syntax::{BinOp, ExportDecl, Expr, ForInit, NewtypeRepr, Pattern, Program, Stmt, Type};

use crate::util::{bin_op_str, escape_string, is_primitive_receiver, un_op_str, write_indent};

fn is_panic_callee(callee: &Expr<'_>) -> bool {
    match callee {
        Expr::Identifier { name: "panic", .. } => true,
        Expr::FieldAccess {
            object,
            field: "panic",
            ..
        } => matches!(object, Expr::Identifier { name: "deka", .. }),
        _ => false,
    }
}

/// Emit JavaScript for a parsed and type-checked program.
pub fn emit_js(program: &Program, _source: &str) -> Result<String, String> {
    emit_js_with_options(
        program,
        _source,
        &HashMap::new(),
        None,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        "module.ds",
        None,
    )
}

/// Emit JavaScript with imported module metadata available.
///
/// Imported structs and enums are seeded into the emitter so that struct
/// literals and enum constructors defined in other modules can be emitted
/// correctly in the current file. `unwrap_calls` maps primitive conversion
/// call sites (`number(x)`, `string(x)`, `bool(x)`) to their lowering kind.
pub fn emit_js_with_imports<'a>(
    program: &'a Program<'a>,
    _source: &str,
    imports: &HashMap<&str, &deka_syntax::ModuleExports<'a>>,
    unwrap_calls: &HashMap<*const Expr<'a>, deka_syntax::typeck::UnwrapKind>,
    operator_rewrites: &HashMap<*const Expr<'a>, deka_syntax::typeck::OperatorRewrite<'a>>,
    method_calls: &HashMap<*const Expr<'a>, deka_syntax::MethodTarget<'a>>,
) -> Result<String, String> {
    emit_js_with_options(
        program,
        _source,
        imports,
        None,
        unwrap_calls,
        operator_rewrites,
        method_calls,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        "module.ds",
        None,
    )
}

/// Emit JavaScript with imported module metadata and a base URL for bare
/// module specifiers.
///
/// When `module_base` is provided, bare import specifiers (those not starting
/// with `.`, `/`, or a URL scheme) are rewritten to
/// `<module_base>/<spec>.mjs`. This lets a host serve stdlib modules as real
/// ESM files instead of string-rewriting compiled output.
pub fn emit_js_with_options<'a>(
    program: &'a Program<'a>,
    _source: &str,
    imports: &HashMap<&str, &deka_syntax::ModuleExports<'a>>,
    module_base: Option<String>,
    unwrap_calls: &HashMap<*const Expr<'a>, deka_syntax::typeck::UnwrapKind>,
    operator_rewrites: &HashMap<*const Expr<'a>, deka_syntax::typeck::OperatorRewrite<'a>>,
    // Primitive extension call sites (`s.slugify()`) to rewrite to
    // free-function calls (`slugify$string(s)`), lowered by the typechecker
    // (deka#527).
    method_calls: &HashMap<*const Expr<'a>, deka_syntax::MethodTarget<'a>>,
    // Builtin `.getType()` call sites to rewrite to `deka.typeOf(x)`,
    // lowered by the typechecker (rfd#41, deka#529).
    type_of_calls: &HashSet<*const Expr<'a>>,
    jsx_optional_props: &HashMap<
        *const deka_syntax::JsxElement<'a>,
        deka_syntax::typeck::JsxOptionalProps<'a>,
    >,
    enum_case_patterns: &HashMap<*const deka_syntax::Pattern<'a>, &'a str>,
    // Union member type-patterns (`string(s)`) and the runtime predicate
    // each one compiles to (rfd#42, deka#530).
    union_type_patterns: &HashMap<
        *const deka_syntax::Pattern<'a>,
        deka_syntax::typeck::UnionMemberTest<'a>,
    >,
    file_path: &str,
    live_names: Option<&HashSet<String>>,
) -> Result<String, String> {
    let mut emitter = Emitter::new(program);
    emitter.module_base = module_base;
    emitter.file_stem = file_stem_from_path(file_path);
    emitter.seed_imports(imports);
    emitter.unwrap_calls = unwrap_calls.clone();
    emitter.operator_rewrites = operator_rewrites.clone();
    emitter.method_calls = method_calls.clone();
    emitter.type_of_calls = type_of_calls.clone();
    emitter.jsx_optional_props = jsx_optional_props.clone();
    emitter.enum_case_patterns = enum_case_patterns.clone();
    emitter.union_type_patterns = union_type_patterns.clone();
    emitter.live_names = live_names.cloned();
    emitter.emit()
}

fn file_stem_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("module")
        .to_string()
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
    receiver_name: String,
    receiver_mutable: bool,
    params: Vec<String>,
    body: Vec<deka_syntax::Stmt<'a>>,
    is_async: bool,
}

struct Emitter<'a> {
    program: &'a Program<'a>,
    out: String,
    uses_struct: bool,
    uses_newtype: bool,
    uses_prelude_enums: bool,
    struct_order: Vec<String>,
    structs: HashMap<String, StructMeta>,
    enums: HashMap<String, EnumMeta>,
    newtypes: HashMap<String, NewtypeRepr>,
    receiver_methods: HashMap<String, Vec<ReceiverMethod<'a>>>,
    /// Base URL for rewriting bare import specifiers.
    module_base: Option<String>,
    /// Primitive conversion calls lowered by the typechecker.
    unwrap_calls: HashMap<*const Expr<'a>, deka_syntax::typeck::UnwrapKind>,
    jsx_optional_props: HashMap<
        *const deka_syntax::JsxElement<'a>,
        deka_syntax::typeck::JsxOptionalProps<'a>,
    >,
    enum_case_patterns: HashMap<*const deka_syntax::Pattern<'a>, &'a str>,
    /// Union member type-patterns and their runtime predicates, lowered by
    /// the typechecker (rfd#42, deka#530).
    union_type_patterns: HashMap<
        *const deka_syntax::Pattern<'a>,
        deka_syntax::typeck::UnionMemberTest<'a>,
    >,
    unwrap_id: usize,
    /// Newtype operator rewrites lowered by the typechecker.
    operator_rewrites: HashMap<*const Expr<'a>, deka_syntax::typeck::OperatorRewrite<'a>>,
    /// Primitive extension call sites lowered by the typechecker to
    /// free-function calls (`slugify$string(s)`) (deka#527).
    method_calls: HashMap<*const Expr<'a>, deka_syntax::MethodTarget<'a>>,
    /// Builtin `.getType()` call sites lowered by the typechecker to
    /// `deka.typeOf(x)` (rfd#41, deka#529).
    type_of_calls: HashSet<*const Expr<'a>>,
    file_stem: String,
    fn_scope: String,
    jsx_path: Vec<usize>,
    jsx_siblings: Vec<usize>,
    jsx_roots: usize,
    /// When set, only these top-level names are emitted (graph shaking).
    live_names: Option<HashSet<String>>,
    needs_live: bool,
}

impl<'a> Emitter<'a> {
    fn new(program: &'a Program<'a>) -> Self {
        let mut emitter = Self {
            program,
            out: String::new(),
            uses_struct: false,
            uses_newtype: false,
            uses_prelude_enums: false,
            struct_order: Vec::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            newtypes: HashMap::new(),
            receiver_methods: HashMap::new(),
            module_base: None,
            unwrap_calls: HashMap::new(),
            jsx_optional_props: HashMap::new(),
            enum_case_patterns: HashMap::new(),
            union_type_patterns: HashMap::new(),
            unwrap_id: 0,
            operator_rewrites: HashMap::new(),
            method_calls: HashMap::new(),
            type_of_calls: HashSet::new(),
            file_stem: "module".to_string(),
            fn_scope: "_".to_string(),
            jsx_path: Vec::new(),
            jsx_siblings: Vec::new(),
            jsx_roots: 0,
            live_names: None,
            needs_live: false,
        };
        emitter.prepass();
        emitter.needs_live = emitter.scan_needs_live();
        emitter
    }

    fn emit(&mut self) -> Result<String, String> {
        // "use strict" belongs in the directive prologue, before even the
        // hoisted imports: after an import it degrades to a dead expression
        // statement, and the kit's RAW display strips it only when it is the
        // first line. (Redundant but legal in ES modules, which are always
        // strict.)
        self.out.push_str("\"use strict\";\n");
        // Imports must precede other statements. Hoist user imports, then the
        // jsx runtime import when this file contains JSX.
        let mut first = true;
        for stmt in self.program.statements.iter() {
            if matches!(stmt, Stmt::Import { .. }) && self.should_emit_stmt(stmt) {
                if !first {
                    self.out.push('\n');
                }
                first = false;
                self.emit_stmt(stmt)?;
            }
        }
        if self.needs_jsx_helper() {
            if !first {
                self.out.push('\n');
            }
            first = false;
            let spec = self.resolve_module_source("ui/jsx");
            self.out.push_str("import { jsx, jsxs, Fragment } from \"");
            self.out.push_str(&spec);
            self.out.push_str("\";");
        }
        if self.needs_live {
            if !first {
                self.out.push('\n');
            }
            first = false;
            let spec = self.resolve_module_source("ui/reactive");
            self.out.push_str("import { live } from \"");
            self.out.push_str(&spec);
            self.out.push_str("\";");
        }

        // Imports end without a trailing newline; separate them from the
        // prelude so each `import` stays on its own line. Hosts transform
        // static imports line-by-line, so `import ...;"use strict";` on one
        // line would survive into non-module execution.
        if !first {
            self.out.push('\n');
        }

        self.emit_prelude()?;

        // First pass: emit struct/enum/function declarations so that all
        // factories exist before receiver methods are registered.
        for stmt in self.program.statements.iter() {
            if matches!(stmt, Stmt::Import { .. }) {
                continue;
            }
            if !Self::is_runtime_statement(stmt) && self.should_emit_stmt(stmt) {
                if !first {
                    self.out.push('\n');
                }
                first = false;
                self.emit_stmt(stmt)?;
            }
        }

        // Register receiver methods after all struct factories are declared.
        self.emit_method_registrations()?;

        // Second pass: emit executable top-level statements (const/let/expr).
        for stmt in self.program.statements.iter() {
            if Self::is_runtime_statement(stmt) && self.should_emit_stmt(stmt) {
                if !first {
                    self.out.push('\n');
                }
                first = false;
                self.emit_stmt(stmt)?;
            }
        }

        Ok(std::mem::take(&mut self.out))
    }

    fn is_live(&self, name: &str) -> bool {
        self.live_names
            .as_ref()
            .map_or(true, |live| live.contains(name))
    }

    fn should_emit_stmt(&self, stmt: &Stmt<'_>) -> bool {
        if self.live_names.is_none() {
            return true;
        }
        match stmt {
            // Same rule as : kept when the bound name is live.
            Stmt::UnwrapLet { name, .. } => self.is_live(name),
            Stmt::Import { specifiers, .. } => {
                specifiers.is_empty() || specifiers.iter().any(|spec| self.is_live(spec.local))
            }
            Stmt::Export { decl, .. } => match decl {
                ExportDecl::Const { name, .. } | ExportDecl::Function { name, .. } => {
                    self.is_live(name)
                }
                ExportDecl::NamedGroup { names } => names.iter().any(|n| {
                    self.is_live(n.alias.unwrap_or(n.name)) || self.is_live(n.name)
                }),
            },
            Stmt::Const { name, .. }
            | Stmt::Let { name, .. }
            | Stmt::Function { name, .. }
            | Stmt::Struct { name, .. }
            | Stmt::Enum { name, .. }
            | Stmt::TypeAlias { name, .. }
            | Stmt::Newtype { name, .. }
            | Stmt::Interface { name, .. } => self.is_live(name),
            Stmt::ReceiverMethod {
                receiver_type,
                name,
                ..
            } => {
                // Primitive extensions are emitted as free functions named
                // `method$receiver`; struct and newtype methods ride the
                // receiver type's liveness (deka#527).
                if is_primitive_receiver(receiver_type) {
                    self.is_live(&format!("{name}${receiver_type}"))
                } else {
                    self.is_live(receiver_type)
                }
            }
            Stmt::Expr { .. }
            | Stmt::If { .. }
            | Stmt::Block { .. }
            | Stmt::For { .. }
            | Stmt::ForOf { .. }
            | Stmt::Return { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Empty { .. } => true,
        }
    }

    /// Returns true for statements whose initializers run at module load time.
    fn is_runtime_statement(stmt: &Stmt<'_>) -> bool {
        matches!(
            stmt,
            Stmt::Const { .. }
                | Stmt::Let { .. }
                // Its initializer runs at load time like any other binding.
                // Omitting it here classified the statement as a declaration,
                // so it was hoisted above the runtime statements and read a
                // binding that had not been initialised yet (deka#445).
                | Stmt::UnwrapLet { .. }
                | Stmt::Expr { .. }
                | Stmt::Return { .. }
                | Stmt::If { .. }
                | Stmt::Block { .. }
                | Stmt::For { .. }
                | Stmt::ForOf { .. }
                | Stmt::Break { .. }
                | Stmt::Continue { .. }
        )
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
                    receiver_name,
                    receiver_mutable,
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
                            receiver_name: receiver_name.to_string(),
                            receiver_mutable: *receiver_mutable,
                            params: params.iter().map(|p| p.name.to_string()).collect(),
                            body: body.to_vec(),
                            is_async: *is_async,
                        });
                }
                Stmt::Newtype { name, repr, .. } => {
                    self.newtypes.insert(name.to_string(), *repr);
                    self.uses_newtype = true;
                }
                _ => {}
            }
        }
        self.compute_empty_embeds();
    }

    /// Rewrite a bare module specifier to a resolvable URL when `module_base`
    /// is configured. Bare specifiers are those that do not start with `.`,
    /// `/`, or a URL scheme. The `@deka/` prefix is stripped so both `io` and
    /// `@deka/io` map to `<base>/io.mjs`.
    ///
    /// Note: unknown bare specifiers are rejected at compile time before emit
    /// when module_base is set (deka#497), so this path only sees stdlib names.
    fn resolve_module_source(&self, source: &str) -> String {
        let Some(base) = &self.module_base else {
            return source.to_string();
        };
        if source.starts_with('.') || source.starts_with('/') {
            return source.to_string();
        }
        if source.contains(':') {
            // URL scheme (e.g. https://, data:)
            return source.to_string();
        }
        let name = source.strip_prefix("@deka/").unwrap_or(source);
        let base = base.trim_end_matches('/');
        format!("{}/{}.mjs", base, name)
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
            for (name, info) in exports.newtypes.iter() {
                if self.newtypes.contains_key(*name) {
                    continue;
                }
                self.newtypes.insert(name.to_string(), info.repr);
                self.uses_newtype = true;
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
        // `type_of_calls` is fully populated by the typechecker before
        // emission, so unlike the emission-time flags above it can gate the
        // helper directly — no AST scan needed. A call in shaken code still
        // forces the ~4-line helper; harmless bloat, never incorrectness.
        let uses_typeof = !self.type_of_calls.is_empty();

        if self.uses_struct || self.uses_newtype {
            self.out.push_str("const __deka = {");
            if self.uses_struct {
                self.out.push_str(r###"Struct:(id,embeds)=>{function f(fields){const o=Object.create(f.prototype);Object.assign(o,fields);Object.defineProperty(o,'__deka_struct',{value:id,enumerable:false,writable:false,configurable:false});return o;}f.id=id;Object.defineProperty(f,'name',{value:id,configurable:true});f.prototype=Object.create(null);f.prototype.constructor=f;f.impl=(a,b)=>{if(typeof a==='string'){const k=a;f.prototype[k]=function(...x){return b.apply(this,x);};}else{for(const k in a)f.prototype[k]=a[k];}return f;};f.implMut=(a,b)=>{if(typeof a==='string'){const k=a;f.prototype[k]=function(...x){if(Object.isFrozen(this))throw new __deka.MutationError(`cannot call mutable method '${k}' on immutable ${id}`);return b.apply(this,x);};}else{for(const k in a){const fn=a[k];f.prototype[k]=function(...x){if(Object.isFrozen(this))throw new __deka.MutationError(`cannot call mutable method '${k}' on immutable ${id}`);return fn.apply(this,x);};}}return f;};if(embeds){for(const [embedName,embedFactory] of Object.entries(embeds)){for(const key of Object.keys(embedFactory.prototype)){f.prototype[key]=function(...args){return this[embedName][key](...args);};}}}return f;},"###);
                self.out.push_str("getStructId:(v)=>v?.__deka_struct,");
                self.out.push_str(
                    "MutationError:class extends Error{constructor(m){super(m);this.name='MutationError';}}",
                );
            }
            if self.uses_newtype {
                if self.uses_struct {
                    self.out.push_str(",");
                }
                self.out.push_str("__nt:Symbol.for('deka.nt')");
            }
            self.out.push_str("};\n");
            self.out.push_str("const deka = globalThis.deka = { ...globalThis.deka, ...__deka };\n");
            if self.uses_newtype {
                self.out.push_str("const __p = deka.__nt;\n");
            }
        }

        // Builtin `.getType()` support (rfd#41, deka#529): a module-local
        // free function, emitted exactly the way deka#527 emits primitive
        // extensions — tree-shakable, no `__deka` prelude, no globalThis.
        // Tags are read directly (`v?.__deka_struct` / `v?.__enum` /
        // `v?.__deka_newtype`): `deka.getStructId` only exists when the
        // struct prelude is emitted, and `.getType()` alone must not force
        // it. Descriptors are interned per (kind, name) and frozen, so `==`
        // on them is identity.
        if uses_typeof {
            self.out.push_str("const __deka_type_cache = new Map();\n");
            self.out.push_str(r###"function __deka_type_of(v){const mk=(k,n)=>{const key=k+":"+n;let t=__deka_type_cache.get(key);if(!t){t=Object.freeze({kind:k,name:n,toString(){return this.name;}});__deka_type_cache.set(key,t);}return t;};if(v===null||v===undefined)return mk("none","none");if(v instanceof Uint8Array)return mk("bytes","bytes");const ty=typeof v;if(ty==="string"||ty==="number"||ty==="boolean"||ty==="function")return mk(ty,ty);if(Array.isArray(v))return mk("array","Array");const nt=v.__deka_newtype;if(nt)return mk("newtype",nt);const st=v.__deka_struct;if(st)return mk("struct",st);const en=v.__enum;if(en)return mk("enum",en);return mk("object","object");}"###);
            self.out.push('\n');
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

    fn enter_jsx_node(&mut self) -> usize {
        let index = if let Some(next) = self.jsx_siblings.last_mut() {
            let i = *next;
            *next += 1;
            i
        } else {
            let i = self.jsx_roots;
            self.jsx_roots += 1;
            i
        };
        self.jsx_path.push(index);
        self.jsx_siblings.push(0);
        index
    }

    fn exit_jsx_node(&mut self) {
        self.jsx_siblings.pop();
        self.jsx_path.pop();
    }

    fn current_deka_id(&self) -> String {
        let path = self
            .jsx_path
            .iter()
            .map(|i| format!("i{i}"))
            .collect::<Vec<_>>()
            .join("/");
        format!("{}:{}/{}", self.file_stem, self.fn_scope, path)
    }

    fn with_fn_scope<T>(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let previous = std::mem::replace(&mut self.fn_scope, name.to_string());
        let prev_roots = self.jsx_roots;
        let prev_path = std::mem::take(&mut self.jsx_path);
        let prev_sibs = std::mem::take(&mut self.jsx_siblings);
        self.jsx_roots = 0;
        let result = f(self);
        self.fn_scope = previous;
        self.jsx_roots = prev_roots;
        self.jsx_path = prev_path;
        self.jsx_siblings = prev_sibs;
        result
    }

    fn needs_struct_helper(&self) -> bool {
        // A struct-member union type-pattern emits `deka.getStructId(...)`,
        // which only exists when the struct prelude block is emitted — even
        // if the struct itself lives in an import and this module declares
        // none (rfd#42, deka#530).
        //
        // Primitive extensions are plain free functions, not prototype
        // registrations, so they must not force the struct prelude (deka#527).
        !self.structs.is_empty()
            || self
                .receiver_methods
                .keys()
                .any(|name| !is_primitive_receiver(name))
            || self
                .union_type_patterns
                .values()
                .any(|t| matches!(t, deka_syntax::typeck::UnionMemberTest::Struct(_)))
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
                // A bare `None` literal is Expr::None, not an EnumConstructor, so it
                // must be detected here or the prelude it now references is not emitted.
                if matches!(expr, Expr::None { .. }) {
                    found = true;
                }
            });
            found
        })
    }

    fn needs_jsx_helper(&self) -> bool {
        self.program.statements.iter().any(|stmt| {
            if !self.should_emit_stmt(stmt) {
                return false;
            }
            let mut found = false;
            visit_stmt_exprs(stmt, &mut |expr| {
                if matches!(expr, Expr::JsxElement { .. } | Expr::JsxFragment { .. }) {
                    found = true;
                }
            });
            found
        })
    }

    fn scan_needs_live(&self) -> bool {
        self.program.statements.iter().any(|stmt| {
            let mut found = false;
            visit_stmt_exprs(stmt, &mut |expr| {
                match expr {
                    Expr::JsxElement { element, .. } => {
                        if element.children.iter().any(jsx_child_needs_live) {
                            found = true;
                        }
                    }
                    Expr::JsxFragment { children, .. } => {
                        if children.iter().any(jsx_child_needs_live) {
                            found = true;
                        }
                    }
                    _ => {}
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
                // Const bindings in DekaScript are immutable. Freeze array and
                // object literals at creation so mutations throw at runtime.
                let needs_freeze = matches!(value, Expr::Array { .. } | Expr::Object { .. });
                if needs_freeze {
                    self.out.push_str("Object.freeze(");
                }
                self.emit_expr(value)?;
                if needs_freeze {
                    self.out.push_str(")");
                }
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
            Stmt::UnwrapLet {
                name,
                is_const,
                scrutinee,
                alternative,
                ..
            } => {
                // Lowered to statements, not an IIFE. That is the whole reason
                // this is a binding form: `return` in the alternative has to
                // leave the enclosing function, and a `return` inside an IIFE
                // returns from the IIFE (deka#445).
                //
                //   let name;
                //   { const __u = scrutinee;
                //     if (__u.__case === "Some") { name = __u.value; }
                //     else { …alternative… } }
                //
                // `let` even for `const`, because the assignment happens in a
                // branch. Reassignment is rejected by typeck, not by JS.
                let _ = is_const;
                let temp = format!("__deka_unwrap_{}", self.next_unwrap_id());
                write_indent(&mut self.out, 0);
                self.out.push_str("let ");
                self.out.push_str(name);
                self.out.push_str(";\n");
                write_indent(&mut self.out, 0);
                self.out.push_str("{\n");
                self.out.push_str(&format!("const {temp} = "));
                self.emit_expr(scrutinee)?;
                self.out.push_str(";\n");
                self.out.push_str(&format!(
                    "if ({temp}.__case === \"Some\" || {temp}.__case === \"Ok\") {{ {name} = {temp}.value; }} else {{\n"
                ));
                match alternative {
                    deka_syntax::UnwrapAlternative::Block(stmts) => {
                        for inner in stmts.iter() {
                            self.emit_stmt_in_unwrap(inner, name)?;
                        }
                    }
                    deka_syntax::UnwrapAlternative::Match(arms) => {
                        // The arms match the original value, so their tests and
                        // bindings are the ordinary ones against the temporary.
                        // The success arm is already handled above.
                        for (index, arm) in arms.iter().enumerate() {
                            let condition = self.match_condition(&arm.pattern, &temp);
                            if index > 0 {
                                self.out.push_str("else ");
                            }
                            self.out.push_str(&format!("if ({condition}) {{\n"));
                            self.emit_pattern_bindings(&arm.pattern, &temp, 0)?;
                            self.out.push_str(&format!("{name} = "));
                            self.emit_expr(&arm.body)?;
                            self.out.push_str(";\n}\n");
                        }
                        if !arms.is_empty() {
                            self.out.push_str(
                                "else { throw new Error(\"non-exhaustive match\"); }\n",
                            );
                        }
                    }
                }
                self.out.push_str("}\n}");
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
                    self.emit_param(param, !is_async)?;
                }
                self.out.push_str(") {\n");
                if *is_async {
                    self.emit_default_param_assignments(params, 1)?;
                }
                self.with_fn_scope(name, |s| {
                    for stmt in body.iter() {
                        s.emit_stmt(stmt)?;
                        s.out.push('\n');
                    }
                    Ok(())
                })?;
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
                            self.emit_param(param, !is_async)?;
                        }
                        self.out.push_str(") {\n");
                        if *is_async {
                            self.emit_default_param_assignments(params, 1)?;
                        }
                        self.with_fn_scope(name, |s| {
                            for stmt in body.iter() {
                                s.emit_stmt(stmt)?;
                                s.out.push('\n');
                            }
                            Ok(())
                        })?;
                        write_indent(&mut self.out, 0);
                        self.out.push('}');
                    }
                    ExportDecl::NamedGroup { names } => {
                        let kept: Vec<_> = names
                            .iter()
                            .filter(|n| {
                                self.is_live(n.alias.unwrap_or(n.name)) || self.is_live(n.name)
                            })
                            .collect();
                        if kept.is_empty() {
                            return Ok(());
                        }
                        self.out.push_str("{ ");
                        for (i, name) in kept.iter().enumerate() {
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
                if source_is_css(source) && specifiers.is_empty() {
                    // Side-effect `import "./x.css"` is collected per-route as <link>.
                    // Keep specifier imports (CSS modules) so the bundler can resolve them.
                    return Ok(());
                }
                write_indent(&mut self.out, 0);
                let resolved_source = self.resolve_module_source(source);
                if specifiers.is_empty() {
                    self.out.push_str("import \"");
                    self.out.push_str(&resolved_source);
                    self.out.push_str("\";");
                } else {
                    let kept: Vec<_> = specifiers
                        .iter()
                        .filter(|spec| self.is_live(spec.local))
                        .collect();
                    if kept.is_empty() {
                        return Ok(());
                    }
                    self.out.push_str("import { ");
                    for (i, spec) in kept.iter().enumerate() {
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
                    self.out.push_str(&resolved_source);
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
            Stmt::Block { body, .. } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("{\n");
                for stmt in body.iter() {
                    self.emit_stmt(stmt)?;
                    self.out.push('\n');
                }
                write_indent(&mut self.out, 0);
                self.out.push('}');
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
            Stmt::ForOf {
                name,
                is_const,
                iterable,
                body,
                ..
            } => {
                write_indent(&mut self.out, 0);
                self.out.push_str("for (");
                if *is_const {
                    self.out.push_str("const ");
                } else {
                    self.out.push_str("let ");
                }
                self.out.push_str(name);
                self.out.push_str(" of ");
                self.emit_expr(iterable)?;
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
            Stmt::Newtype { name, repr, .. } => {
                self.emit_newtype_factory(name, *repr)?;
            }
            Stmt::Interface { .. } => {
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
                self.out.push_str("(value) { return Object.freeze({ ");
                self.out.push_str("__enum: ");
                self.out.push_str(&json_string(name));
                self.out.push_str(", __case: ");
                self.out.push_str(&json_string(&case.name));
                self.out.push_str(", name: ");
                self.out.push_str(&json_string(&case.name));
                self.out.push_str(", value }); }");
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

    fn emit_newtype_factory(
        &mut self,
        name: &str,
        _repr: NewtypeRepr,
    ) -> Result<(), String> {
        write_indent(&mut self.out, 0);
        self.out.push_str("const ");
        self.out.push_str(name);
        self.out.push_str("$proto = Object.create(null);\n");
        write_indent(&mut self.out, 0);
        self.out.push_str("Object.defineProperty(");
        self.out.push_str(name);
        self.out.push_str("$proto, '__deka_newtype', { value: ");
        self.out.push_str(&json_string(name));
        self.out.push_str(", enumerable: false, writable: false, configurable: false });\n");
        write_indent(&mut self.out, 0);
        self.out.push_str(name);
        self.out.push_str("$proto.toJSON = function () { return this[__p]; };\n");
        write_indent(&mut self.out, 0);
        self.out.push_str("function ");
        self.out.push_str(name);
        self.out.push_str("(v) { const o = Object.create(");
        self.out.push_str(name);
        self.out.push_str("$proto); Object.defineProperty(o, __p, { value: v, enumerable: false, writable: false, configurable: false }); return o; }");
        Ok(())
    }

    fn collect_methods_for_struct(
        &self,
        struct_name: &str,
        visited: &mut HashSet<String>,
    ) -> Vec<ReceiverMethod<'a>> {
        if !visited.insert(struct_name.to_string()) {
            return Vec::new();
        }
        let mut methods = Vec::new();
        if let Some(meta) = self.structs.get(struct_name) {
            let embeds = meta.embeds.clone();
            for embed in embeds {
                methods.extend(self.collect_methods_for_struct(&embed, visited));
            }
        }
        if let Some(own) = self.receiver_methods.get(struct_name) {
            methods.extend(own.iter().cloned());
        }
        methods
    }

    fn emit_method_registrations(&mut self) -> Result<(), String> {
        let order = self.struct_order.clone();
        for struct_name in order {
            if !self.is_live(&struct_name) {
                continue;
            }
            let methods = self.collect_methods_for_struct(&struct_name, &mut HashSet::new());
            for method in methods {
                    write_indent(&mut self.out, 0);
                    self.out.push_str(&struct_name);
                    self.out.push_str(if method.receiver_mutable { ".implMut(" } else { ".impl(" });
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
                    write_indent(&mut self.out, 2);
                    self.out.push_str("const ");
                    self.out.push_str(&method.receiver_name);
                    self.out.push_str(" = this;\n");
                    for stmt in method.body.iter() {
                        self.emit_stmt(stmt)?;
                        self.out.push('\n');
                    }
                    write_indent(&mut self.out, 0);
                    self.out.push_str("});\n");
                }
        }

        // Newtype receiver methods are installed directly on the newtype
        // factory's prototype object.
        let newtype_methods: Vec<(String, Vec<ReceiverMethod<'a>>)> = self
            .newtypes
            .keys()
            .filter_map(|name| {
                if !self.is_live(name) {
                    return None;
                }
                self.receiver_methods
                    .get(name)
                    .map(|methods| (name.clone(), methods.clone()))
            })
            .collect();
        for (newtype_name, methods) in newtype_methods {
            for method in methods {
                write_indent(&mut self.out, 0);
                self.out.push_str(&newtype_name);
                self.out.push_str("$proto.");
                self.out.push_str(&method.name);
                self.out.push_str(" = ");
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
                write_indent(&mut self.out, 2);
                self.out.push_str("const ");
                self.out.push_str(&method.receiver_name);
                self.out.push_str(" = this;\n");
                for stmt in method.body.iter() {
                    self.emit_stmt(stmt)?;
                    self.out.push('\n');
                }
                write_indent(&mut self.out, 0);
                self.out.push_str("};\n");
            }
        }

        // Primitive receiver methods cannot hang off a prototype (primitives
        // cannot be branded), so they are emitted as module-local free
        // functions named `method$receiver`; call sites are rewritten to
        // match (deka#527). Unused extensions are dropped by the same
        // liveness gate as everything else.
        let primitive_methods: Vec<(String, Vec<ReceiverMethod<'a>>)> = self
            .receiver_methods
            .iter()
            .filter(|(receiver_type, _)| is_primitive_receiver(receiver_type))
            .map(|(receiver_type, methods)| (receiver_type.clone(), methods.clone()))
            .collect();
        for (receiver_type, methods) in primitive_methods {
            for method in methods {
                let mangled = format!("{}${}", method.name, receiver_type);
                if !self.is_live(&mangled) {
                    continue;
                }
                write_indent(&mut self.out, 0);
                if method.is_async {
                    self.out.push_str("async ");
                }
                self.out.push_str("function ");
                self.out.push_str(&mangled);
                self.out.push('(');
                self.out.push_str(&method.receiver_name);
                for param in method.params.iter() {
                    self.out.push_str(", ");
                    self.out.push_str(param);
                }
                self.out.push_str(") {\n");
                for stmt in method.body.iter() {
                    self.emit_stmt(stmt)?;
                    self.out.push('\n');
                }
                write_indent(&mut self.out, 0);
                self.out.push_str("}\n");
            }
        }
        Ok(())
    }

    fn emit_newtype_binary(
        &mut self,
        rewrite: &deka_syntax::typeck::OperatorRewrite<'a>,
        op: BinOp,
        left: &Expr<'a>,
        right: &Expr<'a>,
    ) -> Result<(), String> {
        use deka_syntax::typeck::{NewtypeSide, OperatorRewrite};
        match rewrite {
            OperatorRewrite::NewtypeBinary { name } => {
                self.out.push_str(name);
                self.out.push_str("(");
                self.out.push_str("(");
                self.emit_expr(left)?;
                self.out.push_str("[__p]");
                self.out.push(' ');
                self.out.push_str(bin_op_str(op));
                self.out.push(' ');
                self.emit_expr(right)?;
                self.out.push_str("[__p]");
                self.out.push_str("))");
            }
            OperatorRewrite::NewtypeDiv => {
                self.out.push_str("(");
                self.emit_expr(left)?;
                self.out.push_str("[__p]");
                self.out.push(' ');
                self.out.push_str(bin_op_str(op));
                self.out.push(' ');
                self.emit_expr(right)?;
                self.out.push_str("[__p])");
            }
            OperatorRewrite::NewtypeScalar { name, side } => {
                self.out.push_str(name);
                self.out.push_str("(");
                self.out.push_str("(");
                match side {
                    NewtypeSide::Left => {
                        self.emit_expr(left)?;
                        self.out.push_str("[__p]");
                        self.out.push(' ');
                        self.out.push_str(bin_op_str(op));
                        self.out.push(' ');
                        self.emit_expr(right)?;
                    }
                    NewtypeSide::Right => {
                        self.emit_expr(left)?;
                        self.out.push(' ');
                        self.out.push_str(bin_op_str(op));
                        self.out.push(' ');
                        self.emit_expr(right)?;
                        self.out.push_str("[__p]");
                    }
                }
                self.out.push_str("))");
            }
            OperatorRewrite::NewtypeCompare => {
                self.out.push_str("(");
                self.emit_expr(left)?;
                self.out.push_str("[__p]");
                self.out.push(' ');
                self.out.push_str(bin_op_str(op));
                self.out.push(' ');
                self.emit_expr(right)?;
                self.out.push_str("[__p])");
            }
            _ => {
                return Err(format!("unexpected unary rewrite for binary expression"));
            }
        }
        Ok(())
    }

    fn emit_newtype_unary(
        &mut self,
        rewrite: &deka_syntax::typeck::OperatorRewrite<'a>,
        op: deka_syntax::UnOp,
        operand: &Expr<'a>,
    ) -> Result<(), String> {
        use deka_syntax::typeck::OperatorRewrite;
        match rewrite {
            OperatorRewrite::NewtypeUnary { name } => {
                self.out.push_str(name);
                self.out.push_str("((");
                self.out.push_str(un_op_str(op));
                self.emit_expr(operand)?;
                self.out.push_str("[__p]))");
            }
            _ => {
                return Err(format!("unexpected binary rewrite for unary expression"));
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

    /// A counter so nested unwraps do not collide on the temporary name.
    fn next_unwrap_id(&mut self) -> usize {
        self.unwrap_id += 1;
        self.unwrap_id
    }

    /// Emit one statement of an `or { … }` block.
    ///
    /// A trailing expression statement is the block's value and is assigned to
    /// the binding; everything else is emitted as-is, so a `return` in there
    /// leaves the enclosing function.
    fn emit_stmt_in_unwrap(
        &mut self,
        stmt: &'a Stmt<'a>,
        binding: &str,
    ) -> Result<(), String> {
        if let Stmt::Expr { expr, .. } = stmt {
            self.out.push_str(&format!("{binding} = "));
            self.emit_expr(expr)?;
            self.out.push_str(";\n");
            return Ok(());
        }
        self.emit_stmt(stmt)?;
        self.out.push('\n');
        Ok(())
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
                // `None` is Option.None, not JS null. The prelude constant and the
                // `__case === "None"` pattern test must agree on one representation
                // (deka#394); emitting bare null made every match on it throw.
                self.out.push_str("Option.None");
            }
            Expr::Identifier { name, .. } => {
                self.out.push_str(name);
            }
            Expr::Binary { op, left, right, .. } => {
                let expr_ptr = expr as *const Expr<'a>;
                let rewrite = self.operator_rewrites.get(&expr_ptr).copied();
                if let Some(rewrite) = rewrite {
                    self.emit_newtype_binary(&rewrite, *op, left, right)?;
                    return Ok(());
                }
                if *op == BinOp::Pipe {
                    // Desugar pipe into a call. The left-hand value becomes
                    // argument 0 unless the right-hand call contains a hole.
                    match right {
                        Expr::Call { callee, args, .. }
                            if !args.iter().any(|a| is_hole_expr(a)) =>
                        {
                            self.emit_expr(callee)?;
                            self.out.push('(');
                            self.emit_expr(left)?;
                            for arg in args.iter() {
                                self.out.push_str(", ");
                                self.emit_expr(arg)?;
                            }
                            self.out.push(')');
                        }
                        _ => {
                            self.out.push('(');
                            self.emit_expr(right)?;
                            self.out.push_str(")(");
                            self.emit_expr(left)?;
                            self.out.push(')');
                        }
                    }
                } else {
                    self.emit_expr(left)?;
                    self.out.push(' ');
                    self.out.push_str(bin_op_str(*op));
                    self.out.push(' ');
                    self.emit_expr(right)?;
                }
            }
            Expr::Unary { op, operand, .. } => {
                let expr_ptr = expr as *const Expr<'a>;
                let rewrite = self.operator_rewrites.get(&expr_ptr).copied();
                if let Some(rewrite) = rewrite {
                    self.emit_newtype_unary(&rewrite, *op, operand)?;
                    return Ok(());
                }
                self.out.push_str(un_op_str(*op));
                self.emit_expr(operand)?;
            }
            Expr::Call { callee, args, span, .. } => {
                let expr_ptr = expr as *const Expr<'a>;
                if let Some(kind) = self.unwrap_calls.get(&expr_ptr) {
                    if let Some(arg) = args.first() {
                        match kind {
                            deka_syntax::typeck::UnwrapKind::Identity => {
                                self.emit_expr(arg)?;
                            }
                            deka_syntax::typeck::UnwrapKind::Payload => {
                                self.emit_expr(arg)?;
                                self.out.push_str("[__p]");
                            }
                            deka_syntax::typeck::UnwrapKind::WidenToString => {
                                self.out.push_str("String(");
                                self.emit_expr(arg)?;
                                self.out.push(')');
                            }
                            deka_syntax::typeck::UnwrapKind::WidenToNumber => {
                                self.out.push_str("Number(");
                                self.emit_expr(arg)?;
                                self.out.push(')');
                            }
                            deka_syntax::typeck::UnwrapKind::WidenToBool => {
                                self.out.push_str("Boolean(");
                                self.emit_expr(arg)?;
                                self.out.push(')');
                            }
                            deka_syntax::typeck::UnwrapKind::StringToOptionNumber => {
                                // `number(s)` on a string can produce NaN;
                                // surface it as Option<number>.
                                self.out.push_str("(() => { const __n = Number(");
                                self.emit_expr(arg)?;
                                self.out.push_str("); return isNaN(__n) ? { __enum: \"Option\", __case: \"None\", name: \"None\" } : { __enum: \"Option\", __case: \"Some\", name: \"Some\", value: __n }; })()");
                            }
                        }
                    }
                    return Ok(());
                }

                // Builtin `.getType()`: rewrite `obj.getType()` to the
                // module-local free function `__deka_type_of(obj)` — static
                // dispatch, no prototype mutation, no globalThis (rfd#41,
                // deka#529).
                if self.type_of_calls.contains(&expr_ptr) {
                    self.out.push_str("__deka_type_of(");
                    if let Expr::FieldAccess { object, .. } = &**callee {
                        self.emit_expr(object)?;
                    }
                    self.out.push(')');
                    return Ok(());
                }

                // Primitive extension call: rewrite `obj.method(args)` to the
                // module-local free function `method$receiver(obj, args)`.
                // Primitives cannot be branded with a prototype, so static
                // dispatch is the only option (deka#527).
                if let Some(target) = self.method_calls.get(&expr_ptr) {
                    self.out.push_str(&target.mangled);
                    self.out.push('(');
                    let mut first = true;
                    if let Expr::FieldAccess { object, .. } = &**callee {
                        self.emit_expr(object)?;
                        first = false;
                    }
                    for arg in args.iter() {
                        if !first {
                            self.out.push_str(", ");
                        }
                        first = false;
                        self.emit_expr(arg)?;
                    }
                    self.out.push(')');
                    return Ok(());
                }

                if is_panic_callee(callee) {
                    self.out.push_str("(() => { throw new Error(String(");
                    if let Some(arg) = args.first() {
                        self.emit_expr(arg)?;
                    } else {
                        self.out.push_str("\"panic\"");
                    }
                    self.out.push_str(")); })()");
                    return Ok(());
                }

                if let Expr::Identifier { name, .. } = callee {
                    if self.newtypes.contains_key(*name) {
                        self.out.push_str(name);
                        self.out.push('(');
                        for (i, arg) in args.iter().enumerate() {
                            if i > 0 {
                                self.out.push_str(", ");
                            }
                            self.emit_expr(arg)?;
                        }
                        self.out.push(')');
                        return Ok(());
                    }
                }

                let hole_count = args.iter().filter(|a| is_hole_expr(a)).count();
                if hole_count > 0 {
                    // Partial application: emit a wrapper function.
                    self.out.push('(');
                    for i in 0..hole_count {
                        if i > 0 {
                            self.out.push_str(", ");
                        }
                        self.out.push_str("__deka_hole_");
                        self.out.push_str(&i.to_string());
                    }
                    self.out.push_str(") => ");
                    self.emit_expr(callee)?;
                    self.out.push('(');
                    let mut hole_idx = 0;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(", ");
                        }
                        if is_hole_expr(arg) {
                            self.out.push_str("__deka_hole_");
                            self.out.push_str(&hole_idx.to_string());
                            hole_idx += 1;
                        } else {
                            self.emit_expr(arg)?;
                        }
                    }
                    self.out.push(')');
                } else {
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
                let _ = span;
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
                        let key = if is_js_identifier(field.key) {
                            field.key.to_string()
                        } else {
                            json_string(field.key)
                        };
                        self.out.push_str(&key);
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
            Expr::Bridge {
                kind,
                action,
                args,
                ..
            } => {
                self.emit_bridge(kind, action, args)?;
            }
            Expr::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.emit_expr(condition)?;
                self.out.push_str(" ? ");
                self.emit_expr(then_branch)?;
                self.out.push_str(" : ");
                self.emit_expr(else_branch)?;
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
                    self.emit_param(param, !is_async)?;
                }
                self.out.push_str(") {\n");
                if *is_async {
                    self.emit_default_param_assignments(params, 1)?;
                }
                self.with_fn_scope("fn", |s| {
                    for stmt in body.iter() {
                        s.emit_stmt(stmt)?;
                        s.out.push('\n');
                    }
                    Ok(())
                })?;
                self.out.push('}');
            }
        }
        Ok(())
    }

    fn emit_param(&mut self, param: &deka_syntax::Param<'a>, emit_default: bool) -> Result<(), String> {
        self.out.push_str(param.name);
        if emit_default {
            if let Some(default) = &param.default_value {
                self.out.push_str(" = ");
                self.emit_expr(default)?;
            }
        }
        Ok(())
    }

    fn emit_default_param_assignments(
        &mut self,
        params: &[deka_syntax::Param<'a>],
        indent: usize,
    ) -> Result<(), String> {
        for param in params.iter() {
            if param.default_value.is_some() {
                write_indent(&mut self.out, indent);
                self.out.push_str("if (");
                self.out.push_str(param.name);
                self.out.push_str(" === undefined) { ");
                self.out.push_str(param.name);
                self.out.push_str(" = ");
                self.emit_expr(param.default_value.as_ref().unwrap())?;
                self.out.push_str("; }\n");
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
        // Fields promoted from embedded structs, recorded as (embed path from
        // this struct to the owner, field name, emitted value).
        let mut promoted: Vec<(Vec<String>, String, String)> = Vec::new();

        for field in fields.iter() {
            seen.insert(field.name.to_string());
            let mut value_buf = String::new();
            std::mem::swap(&mut self.out, &mut value_buf);
            self.emit_expr(&field.value)?;
            std::mem::swap(&mut self.out, &mut value_buf);
            if meta.fields.contains(field.name) || meta.embeds.iter().any(|e| e.as_str() == field.name) {
                let key = if is_js_identifier(field.name) {
                    field.name.to_string()
                } else {
                    json_string(field.name)
                };
                entries.push(format!("{}: {}", key, value_buf));
            } else {
                // Promoted field: route it into the embedded struct that owns
                // it, e.g. `Employee { name: ... }` becomes
                // `Employee({ Person: Person({ name: ... }) })`.
                let mut path = Vec::new();
                if !self.find_promoted_field_path(name, field.name, &mut path) {
                    return Err(format!(
                        "struct `{}` has no field or embed `{}` in emitter",
                        name, field.name
                    ));
                }
                promoted.push((path, field.name.to_string(), value_buf));
            }
        }

        // Auto-fill embedded structs: assemble them from promoted fields when
        // supplied piecemeal, or default-construct them when entirely empty.
        for embed in &meta.embeds {
            if seen.contains(embed) {
                continue;
            }
            let group: Vec<(Vec<String>, String, String)> = promoted
                .iter()
                .filter(|(path, _, _)| path.first() == Some(embed))
                .map(|(path, name, value)| (path[1..].to_vec(), name.clone(), value.clone()))
                .collect();
            if group.is_empty() {
                if meta.empty_embeds.contains(embed) {
                    entries.push(format!("{}: {}({{}})", embed, embed));
                }
                continue;
            }
            let body = self.emit_promoted_embed_body(embed, &group)?;
            entries.push(format!("{}: {}({{ {} }})", embed, embed, body));
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

    /// Find the chain of embedded structs leading from `struct_name` to the
    /// struct that declares `field`. Depth-first, mirroring the typechecker's
    /// promoted-field resolution.
    fn find_promoted_field_path(
        &self,
        struct_name: &str,
        field: &str,
        path: &mut Vec<String>,
    ) -> bool {
        let meta = match self.structs.get(struct_name) {
            Some(m) => m,
            None => return false,
        };
        for embed in &meta.embeds {
            path.push(embed.clone());
            let declares = self
                .structs
                .get(embed)
                .map(|m| m.fields.contains(field))
                .unwrap_or(false);
            if declares || self.find_promoted_field_path(embed, field, path) {
                return true;
            }
            path.pop();
        }
        false
    }

    /// Emit the object-literal body for an embedded struct assembled from
    /// promoted fields. Each entry carries the remaining embed path below
    /// this struct, the field name, and the already-emitted value.
    fn emit_promoted_embed_body(
        &mut self,
        struct_name: &str,
        fields: &[(Vec<String>, String, String)],
    ) -> Result<String, String> {
        let meta = self
            .structs
            .get(struct_name)
            .cloned()
            .ok_or_else(|| format!("unknown struct `{}` in emitter", struct_name))?;
        let mut entries = Vec::new();
        let mut supplied = HashSet::new();
        for (path, name, value) in fields {
            if path.is_empty() {
                supplied.insert(name.clone());
                let key = if is_js_identifier(name) {
                    name.clone()
                } else {
                    json_string(name)
                };
                entries.push(format!("{}: {}", key, value));
            }
        }
        // Recurse into sub-embeds that own any of the remaining fields.
        for embed in &meta.embeds {
            let group: Vec<(Vec<String>, String, String)> = fields
                .iter()
                .filter(|(path, _, _)| path.first() == Some(embed))
                .map(|(path, name, value)| (path[1..].to_vec(), name.clone(), value.clone()))
                .collect();
            if group.is_empty() {
                if meta.empty_embeds.contains(embed) {
                    entries.push(format!("{}: {}({{}})", embed, embed));
                }
                continue;
            }
            let body = self.emit_promoted_embed_body(embed, &group)?;
            entries.push(format!("{}: {}({{ {} }})", embed, embed, body));
        }
        // Auto-fill omitted optional fields, mirroring emit_struct_literal.
        for (opt, default) in &meta.optional {
            if !supplied.contains(opt) {
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
        Ok(entries.join(", "))
    }

    fn emit_unsafe(&mut self, source: &str) -> Result<(), String> {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            self.out.push_str("(function() { try { return { __case: \"Ok\", value: undefined }; } catch (err) { return { __case: \"Err\", error: err instanceof Error ? err : new Error(String(err)) }; } })()");
            return Ok(());
        }

        let is_async = js_has_top_level_await(trimmed);
        let is_statement_block = raw_js_looks_like_statements(trimmed);

        let fn_kw = if is_async { "async function" } else { "function" };
        // The body is raw JavaScript spliced verbatim, so the delimiters that
        // follow it must start on their own line. Without the newlines a body
        // whose last line is a `//` comment swallows the closing `}` and `)()`
        // and the module fails to parse with `Unexpected end of input`
        // (deka#378-adjacent codegen-as-text class; see also #356, #371).
        let inner = if is_statement_block {
            format!("({fn_kw}() {{\n{trimmed}\n}})()")
        } else {
            format!("({fn_kw}() {{ return (\n{trimmed}\n); }})()")
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
        self.out.push_str(" }; } catch (err) { return { __case: \"Err\", error: err instanceof Error ? err : new Error(String(err)) }; } })()");

        Ok(())
    }

    fn emit_bridge(
        &mut self,
        kind: &str,
        action: &str,
        args: &[Expr<'a>],
    ) -> Result<(), String> {
        self.out.push_str("(function() { const __deka_r = __deka_host(");
        self.out.push_str(&json_string(kind));
        self.out.push_str(", ");
        self.out.push_str(&json_string(action));
        self.out.push_str(", [");
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.emit_expr(arg)?;
        }
        self.out.push_str("]); if (__deka_r && __deka_r.ok) { return { __case: \"Ok\", value: __deka_r.value }; } else { return { __case: \"Err\", error: (__deka_r && __deka_r.error) ? __deka_r.error : \"host bridge failed\" }; } })()");
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

    fn match_condition(&mut self, pattern: &Pattern<'a>, scrutinee_var: &str) -> String {
        match pattern {
            Pattern::Wildcard { .. } => "true".to_string(),
            // A bare name that resolved to a payload-free case is a case test,
            // not a binding. Emitting `true` for it is what made every arm
            // after the first one dead (deka#450).
            Pattern::Identifier { .. } => {
                match self
                    .enum_case_patterns
                    .get(&(pattern as *const Pattern<'a>))
                {
                    Some(case) => format!("{}.__case === \"{}\"", scrutinee_var, case),
                    None => "true".to_string(),
                }
            }
            Pattern::Literal { expr, .. } => {
                let mut literal = String::new();
                // Literal patterns are always simple literals; reuse expr emission.
                let mut tmp = Emitter {
                    program: self.program,
                    out: literal,
                    uses_struct: false,
                    uses_newtype: false,
                    uses_prelude_enums: false,
                    struct_order: Vec::new(),
                    structs: HashMap::new(),
                    enums: HashMap::new(),
                    newtypes: HashMap::new(),
                    receiver_methods: HashMap::new(),
                    module_base: self.module_base.clone(),
                    unwrap_calls: HashMap::new(),
                    jsx_optional_props: HashMap::new(),
                    enum_case_patterns: HashMap::new(),
                    union_type_patterns: HashMap::new(),
                    unwrap_id: 0,
                    operator_rewrites: HashMap::new(),
                    method_calls: HashMap::new(),
                    type_of_calls: HashSet::new(),
                    file_stem: self.file_stem.clone(),
                    fn_scope: self.fn_scope.clone(),
                    jsx_path: Vec::new(),
                    jsx_siblings: Vec::new(),
                    jsx_roots: 0,
                    live_names: None,
                    needs_live: false,
                };
                tmp.emit_expr(expr).expect("literal emission");
                literal = tmp.out;
                format!("{} === {}", scrutinee_var, literal)
            }
            Pattern::Constructor { name, payload, .. } => {
                // Union member type-patterns take priority: `string(s)` tests
                // `typeof`, not a `__case` that primitives do not have
                // (rfd#42, deka#530).
                if let Some(test) = self
                    .union_type_patterns
                    .get(&(pattern as *const Pattern<'a>))
                    .copied()
                {
                    return self.union_member_condition(&test, scrutinee_var);
                }
                let mut conditions = vec![format!("{}.__case === \"{}\"", scrutinee_var, name)];
                if let Some(payload) = payload {
                    if let Pattern::Constructor { name: payload_name, .. } = payload {
                        let payload_access = if *name == "Err" {
                            format!("{}.error", scrutinee_var)
                        } else {
                            format!("{}.value", scrutinee_var)
                        };
                        conditions.push(format!("{}.__case === \"{}\"", payload_access, payload_name));
                    }
                }
                conditions.join(" && ")
            }
            // Alternatives cannot bind (deka#446), so the test is a plain
            // disjunction and there is nothing to destructure.
            Pattern::Or { alternatives, .. } => {
                let tests: Vec<String> = alternatives
                    .iter()
                    .map(|alternative| self.match_condition(alternative, scrutinee_var))
                    .collect();
                format!("({})", tests.join(" || "))
            }
            Pattern::Struct { .. } | Pattern::Tuple { .. } => "false".to_string(),
        }
    }

    /// The runtime predicate for a union member type-pattern (rfd#42,
    /// deka#530). Structs force the struct prelude block — `getStructId`
    /// only exists when it is emitted.
    fn union_member_condition(
        &mut self,
        test: &deka_syntax::typeck::UnionMemberTest<'a>,
        scrutinee_var: &str,
    ) -> String {
        match test {
            deka_syntax::typeck::UnionMemberTest::Primitive(name) => {
                let js_type = if *name == "void" { "undefined" } else { name };
                format!("typeof {} === \"{}\"", scrutinee_var, js_type)
            }
            deka_syntax::typeck::UnionMemberTest::Bytes => {
                format!("{} instanceof Uint8Array", scrutinee_var)
            }
            deka_syntax::typeck::UnionMemberTest::Struct(name) => {
                self.uses_struct = true;
                format!("deka.getStructId({}) === \"{}\"", scrutinee_var, name)
            }
            deka_syntax::typeck::UnionMemberTest::Enum(name) => {
                format!("{}.__enum === \"{}\"", scrutinee_var, name)
            }
        }
    }

    fn emit_pattern_bindings(
        &mut self,
        pattern: &Pattern<'a>,
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
                        // Alternatives cannot bind (deka#446), so there is nothing to
            // destructure here.
            Pattern::Or { .. } => {}
            Pattern::Constructor { name, payload, .. } => {
                // Union type-patterns: the payload IS the scrutinee, so the
                // binding is `const s = <scrutinee>;` — the Identifier arm
                // below, reached by passing the scrutinee through unchanged
                // (rfd#42, deka#530).
                if self
                    .union_type_patterns
                    .contains_key(&(pattern as *const Pattern<'a>))
                {
                    if let Some(payload) = payload {
                        self.emit_pattern_bindings(payload, scrutinee_var, indent)?;
                    }
                    return Ok(());
                }
                if let Some(payload) = payload {
                    let payload_access = if *name == "None" {
                        scrutinee_var.to_string()
                    } else if *name == "Err" {
                        format!("{}.error", scrutinee_var)
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
        self.enter_jsx_node();
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

        // Cloned rather than borrowed: emitting an attribute value needs
        // `&mut self`, and the plan is two small Vecs of names.
        let plan = self
            .jsx_optional_props
            .get(&(element as *const deka_syntax::JsxElement<'a>))
            .cloned();
        let plan = plan.as_ref();

        let mut props = Vec::new();
        if !is_component {
            props.push(format!(
                "\"data-deka-id\": {}",
                json_string(&self.current_deka_id())
            ));
        }
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
                // An `?:` prop is `Option<T>` inside the component and the
                // caller wrote a bare `T`. This is a construction site the
                // compiler owns, so it does the wrapping (deka#416).
                let value = if plan
                    .map(|plan| plan.wrap_some.iter().any(|name| *name == attr.name))
                    .unwrap_or(false)
                {
                    self.uses_prelude_enums = true;
                    format!("Option.Some({value})")
                } else {
                    value
                };
                props.push(format!("\"{}\": {}", escape_string(attr.name), value));
            }
        }

        // Optional props the caller left out. Without this the field would be
        // JS `undefined` while the type says `Option<T>` -- the runtime hole
        // deka#401 closed for `T?`.
        if let Some(plan) = plan {
            for name in plan.fill_none.iter() {
                self.uses_prelude_enums = true;
                props.push(format!("\"{}\": Option.None", escape_string(name)));
            }
        }

        let mut child_values = Vec::new();
        for child in element.children.iter() {
            child_values.push(self.emit_jsx_child(child)?);
        }

        if !child_values.is_empty() {
            if child_values.len() == 1 {
                props.push(format!("\"children\": {}", child_values[0]));
            } else {
                props.push(format!("\"children\": [{}]", child_values.join(", ")));
            }
        }

        let fn_name = if child_values.len() > 1 { "jsxs" } else { "jsx" };
        self.out.push_str(fn_name);
        self.out.push('(');
        self.out.push_str(&tag_expr);
        self.out.push_str(", {");
        self.out.push_str(&props.join(", "));
        self.out.push_str("})");
        self.exit_jsx_node();
        Ok(())
    }

    fn emit_jsx_child(&mut self, child: &Expr<'a>) -> Result<String, String> {
        let mut buf = String::new();
        std::mem::swap(&mut self.out, &mut buf);
        if jsx_child_needs_live(child) {
            self.out.push_str("live(function() { return ");
            self.emit_expr(child)?;
            self.out.push_str("; })");
        } else {
            self.emit_expr(child)?;
        }
        std::mem::swap(&mut self.out, &mut buf);
        Ok(buf)
    }

    fn emit_jsx_fragment(&mut self, children: &[Expr<'a>]) -> Result<(), String> {
        self.enter_jsx_node();
        let mut child_values = Vec::new();
        for child in children.iter() {
            child_values.push(self.emit_jsx_child(child)?);
        }

        let fn_name = if child_values.len() > 1 { "jsxs" } else { "jsx" };
        self.out.push_str(fn_name);
        self.out.push('(');
        self.out.push_str("Fragment, {");
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
        self.exit_jsx_node();
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

fn is_hole_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Identifier { name: "_", .. })
}

fn is_builtin_call<'a>(callee: &Expr<'a>, name: &str) -> bool {
    matches!(callee, Expr::Identifier { name: n, .. } if *n == name)
}

#[derive(Default)]
struct RawJsScan<'a> {
    first_word: Option<&'a str>,
    has_top_level_semicolon: bool,
    has_await: bool,
    saw_significant: bool,
    previous_can_end_expression: bool,
}

#[derive(Clone, Copy)]
enum RawJsTokenClass {
    Word,
    Number,
    String,
    Template,
    Punctuation(u8),
}

/// Scan the parts of an unsafe body that affect wrapper selection.
///
/// This is deliberately a small lexical scanner rather than a JavaScript
/// parser. The browser compiler cannot afford to link SWC's parser, but the
/// old string searches were not safe: punctuation and keywords inside
/// literals, comments, and regexes changed the emitted wrapper. Keeping this
/// scanner lexical also means native and WASM compilers make the same choice.
fn scan_raw_js(raw: &str) -> RawJsScan<'_> {
    let bytes = raw.as_bytes();
    let mut scan = RawJsScan::default();
    let mut i = 0;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut line_break_since_token = false;
    let mut previous_allows_regex = true;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                if bytes[i] == b'\n' {
                    line_break_since_token = true;
                }
                i += 1;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                line_break_since_token = true;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                let start = i;
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                if i + 1 < bytes.len() {
                    i += 2;
                }
                if raw[start..i.min(raw.len())].contains('\n') {
                    line_break_since_token = true;
                }
            }
            b'\'' | b'"' => {
                let start = i;
                i = skip_js_quoted(raw, i);
                scan_token(
                    &mut scan,
                    &raw[start..i],
                    RawJsTokenClass::String,
                    line_break_since_token,
                    paren_depth,
                    bracket_depth,
                    brace_depth,
                );
                line_break_since_token = false;
                previous_allows_regex = false;
            }
            b'`' => {
                let start = i;
                i = skip_js_template(raw, i);
                scan_token(
                    &mut scan,
                    &raw[start..i],
                    RawJsTokenClass::Template,
                    line_break_since_token,
                    paren_depth,
                    bracket_depth,
                    brace_depth,
                );
                line_break_since_token = false;
                previous_allows_regex = false;
            }
            b'/' if previous_allows_regex => {
                i = skip_js_regex(raw, i);
                scan_token(
                    &mut scan,
                    "/regex/",
                    RawJsTokenClass::String,
                    line_break_since_token,
                    paren_depth,
                    bracket_depth,
                    brace_depth,
                );
                line_break_since_token = false;
                previous_allows_regex = false;
            }
            b'0'..=b'9' => {
                let start = i;
                i = skip_js_number(bytes, i);
                scan_token(
                    &mut scan,
                    &raw[start..i],
                    RawJsTokenClass::Number,
                    line_break_since_token,
                    paren_depth,
                    bracket_depth,
                    brace_depth,
                );
                line_break_since_token = false;
                previous_allows_regex = false;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
                {
                    i += 1;
                }
                let word = &raw[start..i];
                scan_token(
                    &mut scan,
                    word,
                    RawJsTokenClass::Word,
                    line_break_since_token,
                    paren_depth,
                    bracket_depth,
                    brace_depth,
                );
                line_break_since_token = false;
                previous_allows_regex = word_allows_regex_after(word);
            }
            punct => {
                let start = i;
                let class = RawJsTokenClass::Punctuation(punct);
                i += 1;
                if i < bytes.len() && is_two_byte_js_punctuation(punct, bytes[i]) {
                    i += 1;
                }
                scan_token(
                    &mut scan,
                    &raw[start..i],
                    class,
                    line_break_since_token,
                    paren_depth,
                    bracket_depth,
                    brace_depth,
                );
                line_break_since_token = false;
                match punct {
                    b'(' => paren_depth += 1,
                    b')' => paren_depth = paren_depth.saturating_sub(1),
                    b'[' => bracket_depth += 1,
                    b']' => bracket_depth = bracket_depth.saturating_sub(1),
                    b'{' => brace_depth += 1,
                    b'}' => brace_depth = brace_depth.saturating_sub(1),
                    _ => {}
                }
                previous_allows_regex = punctuation_allows_regex_after(punct);
            }
        }
    }

    scan
}

fn scan_token<'a>(
    scan: &mut RawJsScan<'a>,
    text: &'a str,
    class: RawJsTokenClass,
    line_break_before: bool,
    paren_depth: usize,
    bracket_depth: usize,
    brace_depth: usize,
) {
    let at_top_level = paren_depth == 0 && bracket_depth == 0 && brace_depth == 0;
    let can_start_expression = matches!(class, RawJsTokenClass::Word | RawJsTokenClass::Number | RawJsTokenClass::String | RawJsTokenClass::Template);
    if line_break_before
        && at_top_level
        && scan.previous_can_end_expression
        && can_start_expression
    {
        scan.has_top_level_semicolon = true;
    }

    if !scan.saw_significant {
        if let RawJsTokenClass::Word = class {
            scan.first_word = Some(text);
        }
        scan.saw_significant = true;
    }
    if text == "await" {
        scan.has_await = true;
    }
    if let RawJsTokenClass::Punctuation(punct) = class {
        if punct == b';' && at_top_level {
            scan.has_top_level_semicolon = true;
        }
    }
    scan.previous_can_end_expression = matches!(
        class,
        RawJsTokenClass::Word
            | RawJsTokenClass::Number
            | RawJsTokenClass::String
            | RawJsTokenClass::Template
    );
    if let RawJsTokenClass::Punctuation(punct) = class {
        scan.previous_can_end_expression |= matches!(punct, b')' | b']' | b'}');
    }
}

fn skip_js_quoted(raw: &str, mut i: usize) -> usize {
    let quote = raw.as_bytes()[i];
    i += 1;
    let bytes = raw.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
        } else if bytes[i] == quote {
            return i + 1;
        } else {
            i += 1;
        }
    }
    i
}

fn skip_js_template(raw: &str, mut i: usize) -> usize {
    let bytes = raw.as_bytes();
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
        } else if bytes[i] == b'`' {
            return i + 1;
        } else {
            i += 1;
        }
    }
    i
}

fn skip_js_regex(raw: &str, mut i: usize) -> usize {
    let bytes = raw.as_bytes();
    i += 1;
    let mut in_class = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b'[' => {
                in_class = true;
                i += 1;
            }
            b']' => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return i;
            }
            _ => i += 1,
        }
    }
    i
}

fn skip_js_number(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'.' | b'_')) {
        i += 1;
    }
    i
}

fn word_allows_regex_after(word: &str) -> bool {
    matches!(
        word,
        "return"
            | "throw"
            | "case"
            | "delete"
            | "void"
            | "typeof"
            | "instanceof"
            | "in"
            | "of"
            | "yield"
            | "await"
    )
}

fn punctuation_allows_regex_after(punct: u8) -> bool {
    matches!(
        punct,
        b'(' | b'[' | b'{' | b'=' | b':' | b',' | b';' | b'!' | b'?' | b'&' | b'|'
            | b'+' | b'-' | b'*' | b'%' | b'^' | b'~' | b'<' | b'>'
    )
}

fn is_two_byte_js_punctuation(first: u8, second: u8) -> bool {
    matches!((first, second), (b'=', b'=') | (b'!', b'=') | (b'&', b'&') | (b'|', b'|') | (b'=', b'>') | (b'+', b'+') | (b'-', b'-') | (b'<', b'=') | (b'>', b'='))
}

fn raw_js_looks_like_statements(raw: &str) -> bool {
    let scan = scan_raw_js(raw);
    scan.has_top_level_semicolon
        || matches!(
            scan.first_word,
            Some(
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
        )
}

fn js_has_top_level_await(raw: &str) -> bool {
    scan_raw_js(raw).has_await
}

fn source_is_css(source: &str) -> bool {
    let trimmed = source.trim().trim_matches('"').trim_matches('\'');
    let lower = trimmed.to_ascii_lowercase();
    lower.ends_with(".css")
}

fn expr_contains_jsx(expr: &Expr) -> bool {
    let mut found = false;
    visit_expr(expr, &mut |e| {
        if matches!(e, Expr::JsxElement { .. } | Expr::JsxFragment { .. }) {
            found = true;
        }
    });
    found
}

fn jsx_child_needs_live(expr: &Expr) -> bool {
    match expr {
        Expr::String { .. }
        | Expr::Number { .. }
        | Expr::Boolean { .. }
        | Expr::JsxText { .. }
        | Expr::JsxElement { .. }
        | Expr::JsxFragment { .. }
        | Expr::None { .. } => false,
        Expr::Paren { expr, .. } => jsx_child_needs_live(expr),
        _ if expr_contains_jsx(expr) => false,
        _ => true,
    }
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
        | Stmt::For { body, .. }
        | Stmt::ForOf { body, .. } => {
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
        Stmt::Block { body, .. } => {
            for s in body.iter() {
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
