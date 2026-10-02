//! DSC is linked as a library; no JS emission or compiler subprocess.
use crate::{Function, Hosts, Literal, Op, Program, Result};
use deka_syntax::{Diagnostic, Severity, ast::*};
use std::collections::{BTreeMap, HashMap};

pub fn compile(source: &str, hosts: &Hosts) -> Result<Program> {
    compile_entry(source, hosts, "main")
}
/// Compile an in-memory module with a required entry function.
pub fn compile_entry(source: &str, hosts: &Hosts, entry_name: &str) -> Result<Program> {
    compile_modules(
        &[(std::path::PathBuf::from("<source>"), source.to_owned())],
        hosts,
        Some(entry_name),
    )
}
/// Compile a source file and its relative modules, once each, in dependency order.
/// Import cycles load with JavaScript module semantics (deka#1206): a module
/// already being loaded is not loaded again, and reading one of its exports
/// before it finishes initializing is a named error, not a silent value.
/// Self-imports and external packages fail explicitly.
pub fn compile_file(path: &std::path::Path, hosts: &Hosts, entry: Option<&str>) -> Result<Program> {
    compile_modules(&load_modules(path)?, hosts, entry)
}
/// Watch dependencies even while an imported file is absent or being edited.
/// Compilation still fails closed; this list only controls development reload.
pub fn source_files(path: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    fn visit(path: std::path::PathBuf, files: &mut std::collections::BTreeSet<std::path::PathBuf>) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if !files.insert(path.clone()) {
            return;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            return;
        };
        let arena = bumpalo::Bump::new();
        let parsed = deka_syntax::parse(&source, &arena);
        if let Some(ast) = parsed.program {
            for stmt in ast.statements {
                if let Stmt::Import { source, .. } = stmt
                    && (source.starts_with("./") || source.starts_with("../"))
                    && let Some(parent) = path.parent()
                {
                    visit(parent.join(source), files);
                }
            }
        }
    }
    let mut files = Default::default();
    visit(
        std::fs::canonicalize(path).map_err(|e| e.to_string())?,
        &mut files,
    );
    Ok(files.into_iter().collect())
}
pub fn test_entries(path: &std::path::Path) -> Result<Vec<String>> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let arena = bumpalo::Bump::new();
    let parsed = deka_syntax::parse(&source, &arena);
    diagnostics(&parsed.errors)?;
    let ast = parsed.program.ok_or("missing source program")?;
    Ok(ast
        .statements
        .iter()
        .filter_map(|s| match s {
            Stmt::Function { name, params, .. }
            | Stmt::Export {
                decl: ExportDecl::Function { name, params, .. },
                ..
            } if name.starts_with("test_") && params.is_empty() => Some((*name).to_owned()),
            _ => None,
        })
        .collect())
}
fn host_module(source: &str) -> bool {
    matches!(source, "vm:host" | "io" | "test")
}
fn module_path(parent: &std::path::Path, source: &str) -> Result<std::path::PathBuf> {
    if !source.starts_with("./") && !source.starts_with("../") {
        return Err(format!(
            "unsupported module: {source}; use a relative .ds/.dsx path or a built-in module"
        ));
    }
    let path = parent.parent().ok_or("module has no parent")?.join(source);
    std::fs::canonicalize(&path).map_err(|e| format!("{}: {e}", path.display()))
}
/// Modules in dependency order. A module already being loaded is not loaded
/// again (import cycles load, deka#1206); reads of exports that initialize
/// later than the importer become checked loads during lowering, computed
/// from the loaded order in `compile_modules`. Self-imports are refused.
fn load_modules(path: &std::path::Path) -> Result<Vec<(std::path::PathBuf, String)>> {
    fn visit(
        path: std::path::PathBuf,
        visiting: &mut std::collections::BTreeSet<std::path::PathBuf>,
        loaded: &mut Vec<(std::path::PathBuf, String)>,
    ) -> Result<()> {
        if loaded.iter().any(|(p, _)| *p == path) {
            return Ok(());
        }
        if !visiting.insert(path.clone()) {
            return Ok(());
        }
        let source =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let arena = bumpalo::Bump::new();
        let parsed = deka_syntax::parse(&source, &arena);
        diagnostics(&parsed.errors).map_err(|e| format!("{}: {e}", path.display()))?;
        for stmt in parsed.program.ok_or("missing source program")?.statements {
            if let Stmt::Import { source, .. } = stmt
                && !host_module(source)
            {
                let target = module_path(&path, source)?;
                if target == path {
                    return Err(format!("cyclic module import: {}", path.display()));
                }
                if !visiting.contains(&target) {
                    visit(target, visiting, loaded)?;
                }
            }
        }
        visiting.remove(&path);
        loaded.push((path, source));
        Ok(())
    }
    let mut loaded = vec![];
    visit(
        std::fs::canonicalize(path).map_err(|e| e.to_string())?,
        &mut Default::default(),
        &mut loaded,
    )?;
    Ok(loaded)
}
/// One module lowered into the shared entry function. Returns the module's
/// export-name → slot map. `forward` holds this module's cycle-closing
/// imports (local → (exported name, target)); in harvest mode
/// (`mark_checked = false`) those alias a placeholder local so body lowering
/// resolves, in the real pass they alias the exporter's slot and their reads
/// become checked loads (deka#1206).
#[allow(clippy::too_many_arguments)]
fn lower_module(
    path: &std::path::Path,
    ast: &deka_syntax::ast::Program<'_>,
    source: &str,
    hosts: &Hosts,
    host_exports: &deka_syntax::ModuleExports<'_>,
    module_exports: &HashMap<std::path::PathBuf, deka_syntax::ModuleExports<'_>>,
    bindings: &HashMap<std::path::PathBuf, BTreeMap<String, usize>>,
    forward: Option<&BTreeMap<String, (String, std::path::PathBuf)>>,
    mark_checked: bool,
    entry: &mut Context,
    lower: &mut Lower,
) -> Result<BTreeMap<String, usize>> {
    let mut imports = HashMap::new();
    for stmt in ast.statements {
        if let Stmt::Import { source, .. } = stmt {
            let exports = if host_module(source) {
                host_exports
            } else {
                module_exports
                    .get(&module_path(path, source)?)
                    .ok_or("module was not loaded")?
            };
            imports.insert(*source, exports);
        }
    }
    diagnostics(&deka_syntax::check_program_with_imports(ast, source, &imports).errors)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    entry.names.clear();
    entry.checked.clear();
    lower.hosts.clear();
    for stmt in ast.statements {
        if let Stmt::Import {
            source, specifiers, ..
        } = stmt
        {
            for spec in *specifiers {
                if spec.is_type_only {
                    return Err("type-only imports are not supported by the native VM".into());
                }
                if host_module(source) {
                    hosts.operation(spec.imported)?;
                    lower.hosts.insert(spec.local.into(), spec.imported.into());
                    continue;
                }
                let target = module_path(path, source)?;
                if let Some((imported, cycle_target)) =
                    forward.and_then(|f| f.get(spec.local).map(|f| (&f.0, &f.1)))
                {
                    debug_assert_eq!(imported.as_str(), spec.imported);
                    debug_assert_eq!(cycle_target, &target);
                    // Reserve a local in both passes so the harvest and real
                    // allocation sequences stay identical.
                    entry.bind(spec.local);
                    if mark_checked {
                        let slot = bindings
                            .get(&target)
                            .and_then(|b| b.get(spec.imported))
                            .ok_or("missing module export")?;
                        entry.names.insert(spec.local.into(), *slot);
                        entry.checked.insert(
                            spec.local.into(),
                            format!(
                                "export `{}` of `{}` is not initialized yet (import cycle with `{}`)",
                                spec.imported,
                                target.display(),
                                path.display()
                            ),
                        );
                    }
                } else {
                    let slot = bindings
                        .get(&target)
                        .and_then(|b| b.get(spec.imported))
                        .ok_or("missing module export")?;
                    entry.names.insert(spec.local.into(), *slot);
                }
            }
        }
    }
    let mut exported = BTreeMap::new();
    for stmt in ast.statements {
        lower.statement(stmt, entry)?;
        if let Stmt::Export { decl, .. } = stmt {
            match decl {
                ExportDecl::Function {
                    name, is_default, ..
                } => {
                    exported.insert(
                        if *is_default { "default" } else { name }.to_string(),
                        entry.slot(name)?,
                    );
                }
                ExportDecl::Const { name, .. } => {
                    exported.insert((*name).to_string(), entry.slot(name)?);
                }
                _ => {
                    return Err(
                        "native module exports currently support functions and constants".into(),
                    );
                }
            }
        }
    }
    Ok(exported)
}

fn compile_modules(
    modules: &[(std::path::PathBuf, String)],
    hosts: &Hosts,
    entry_name: Option<&str>,
) -> Result<Program> {
    let arena = bumpalo::Bump::new();
    let declarations = hosts.declarations();
    let host_parse = deka_syntax::parse(&declarations, &arena);
    diagnostics(&host_parse.errors)?;
    let host_ast = host_parse.program.ok_or("missing host declarations")?;
    let host_exports = deka_syntax::collect_module_exports(&host_ast, &arena);
    // Parse every module up front so import checks resolve across a cycle.
    let mut asts = HashMap::new();
    let mut module_exports = HashMap::new();
    for (path, source) in modules {
        let parsed = deka_syntax::parse(source, &arena);
        diagnostics(&parsed.errors)?;
        let ast: &deka_syntax::ast::Program<'_> =
            arena.alloc(parsed.program.ok_or("missing source program")?);
        module_exports.insert(
            path.clone(),
            deka_syntax::collect_module_exports(ast, &arena),
        );
        asts.insert(path.clone(), ast);
    }
    // The relative-import graph: importer → (local, exported name, target).
    let mut edges: HashMap<std::path::PathBuf, Vec<(String, String, std::path::PathBuf)>> =
        HashMap::new();
    for (path, _source) in modules {
        for stmt in asts[path].statements {
            if let Stmt::Import {
                source, specifiers, ..
            } = stmt
                && !host_module(source)
            {
                let target = module_path(path, source)?;
                for spec in *specifiers {
                    edges.entry(path.clone()).or_default().push((
                        spec.local.into(),
                        spec.imported.into(),
                        target.clone(),
                    ));
                }
            }
        }
    }
    // A module is a cycle member when it can reach itself through the graph.
    let mut members: std::collections::BTreeSet<std::path::PathBuf> = Default::default();
    for (path, _source) in modules {
        let mut seen: std::collections::BTreeSet<std::path::PathBuf> = Default::default();
        let mut queue = vec![path.clone()];
        while let Some(module) = queue.pop() {
            for (_local, _imported, target) in edges.get(&module).cloned().unwrap_or_default() {
                if target == *path {
                    members.insert(path.clone());
                }
                if seen.insert(target.clone()) {
                    queue.push(target);
                }
            }
        }
    }
    // An import whose target initializes after the importer (later in loaded
    // order) is a forward import: its reads become checked loads.
    let position: HashMap<std::path::PathBuf, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, (path, _))| (path.clone(), i))
        .collect();
    let mut forward: HashMap<std::path::PathBuf, BTreeMap<String, (String, std::path::PathBuf)>> =
        HashMap::new();
    for (module, specifiers) in &edges {
        for (local, imported, target) in specifiers {
            if position[target] > position[module] {
                forward
                    .entry(module.clone())
                    .or_default()
                    .insert(local.clone(), (imported.clone(), target.clone()));
            }
        }
    }
    let mut bindings: HashMap<std::path::PathBuf, BTreeMap<String, usize>> = HashMap::new();
    let mut lower = Lower {
        functions: vec![],
        hosts: BTreeMap::new(),
    };
    let mut entry = Context::new("<entry>", false);
    lower.functions.push(entry.function.clone());
    let mut last_async = false;
    let mut top_async = false;
    let mut harvested: HashMap<std::path::PathBuf, BTreeMap<String, usize>> = HashMap::new();
    for (path, source) in modules {
        if harvested.is_empty() && members.contains(path) {
            // Pre-lower every cycle member into a scratch entry to learn its
            // export slots, then roll back. The real pass follows the same
            // allocation sequence (verified below), so the slots line up and
            // forward imports resolve before the exporter's code runs.
            let snapshot = entry.clone();
            let functions_len = lower.functions.len();
            let mut sentinel_slots = vec![];
            // Members harvested earlier in this pass resolve for later ones.
            let mut scratch_bindings = bindings.clone();
            for (member, member_source) in modules {
                if !members.contains(member) {
                    continue;
                }
                let exported = lower_module(
                    member,
                    asts[member],
                    member_source,
                    hosts,
                    &host_exports,
                    &module_exports,
                    &scratch_bindings,
                    forward.get(member),
                    false,
                    &mut entry,
                    &mut lower,
                )?;
                sentinel_slots.extend(exported.values().copied());
                harvested.insert(member.clone(), exported.clone());
                scratch_bindings.insert(member.clone(), exported);
            }
            entry = snapshot;
            lower.functions.truncate(functions_len);
            for slot in sentinel_slots {
                entry.emit(Op::Const(Literal::Uninitialized));
                entry.emit(Op::Store(slot));
            }
            for (member, exported) in &harvested {
                bindings.insert(member.clone(), exported.clone());
            }
        }
        let ast = asts[path];
        let exported = lower_module(
            path,
            ast,
            source,
            hosts,
            &host_exports,
            &module_exports,
            &bindings,
            forward.get(path),
            true,
            &mut entry,
            &mut lower,
        )?;
        if let Some(expected) = harvested.get(path)
            && *expected != exported
        {
            return Err(format!(
                "import-cycle slot drift while lowering {}",
                path.display()
            ));
        }
        bindings.insert(path.clone(), exported);
        top_async |= ast.has_top_level_await;
        if let Some(name) = entry_name {
            last_async = ast.statements.iter().any(|s| match s {
                Stmt::Function {
                    name: n, is_async, ..
                }
                | Stmt::Export {
                    decl:
                        ExportDecl::Function {
                            name: n, is_async, ..
                        },
                    ..
                } => *n == name && *is_async,
                _ => false,
            });
        }
    }
    if let Some(name) = entry_name {
        let slot = entry
            .slot(name)
            .map_err(|_| format!("source requires fn {name}()"))?;
        entry.emit(Op::Load(slot));
        entry.emit(Op::Call(0));
        if last_async {
            entry.emit(Op::Await);
        }
    } else {
        entry.emit(Op::Const(Literal::Unit));
    }
    entry.function.asynchronous = top_async || last_async;
    entry.emit(Op::Return);
    lower.functions[0] = entry.function;
    let program = Program {
        version: 1,
        functions: lower.functions,
    };
    program.validate()?;
    Ok(program)
}
fn diagnostics(items: &[Diagnostic]) -> Result<()> {
    let errors = items
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}:{}: {}", d.line, d.column, d.message))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
#[derive(Clone)]
struct Context {
    function: Function,
    names: BTreeMap<String, usize>,
    /// Names whose reads must check for the `Uninitialized` sentinel at
    /// runtime: imports across a cycle-closing edge, mapped to the error
    /// naming the export and both files (deka#1206).
    checked: BTreeMap<String, String>,
    loop_depth: usize,
}
impl Context {
    fn new(name: &str, asynchronous: bool) -> Self {
        Self {
            function: Function {
                name: name.into(),
                parameters: 0,
                captures: 0,
                locals: 0,
                asynchronous,
                code: vec![],
            },
            names: BTreeMap::new(),
            checked: BTreeMap::new(),
            loop_depth: 0,
        }
    }
    fn bind(&mut self, name: &str) -> usize {
        let i = self.function.locals;
        self.function.locals += 1;
        self.names.insert(name.into(), i);
        // A fresh binding shadows any cycle-checked import of the same name.
        self.checked.remove(name);
        i
    }
    fn emit_load(&mut self, name: &str) -> Result<()> {
        let slot = self.slot(name)?;
        if let Some(message) = self.checked.get(name) {
            let message = message.clone();
            self.emit(Op::LoadChecked { slot, message });
        } else {
            self.emit(Op::Load(slot));
        }
        Ok(())
    }
    fn slot(&self, name: &str) -> Result<usize> {
        self.names.get(name).copied().ok_or_else(|| {
            format!("binding {name} is unavailable here; forward references are unsupported")
        })
    }
    fn emit(&mut self, op: Op) -> usize {
        let i = self.function.code.len();
        self.function.code.push(op);
        i
    }
    fn patch(&mut self, at: usize) {
        let end = self.function.code.len();
        match &mut self.function.code[at] {
            Op::Jump(i) | Op::JumpIfFalse(i) => *i = end,
            _ => unreachable!(),
        }
    }
}
struct Lower {
    functions: Vec<Function>,
    hosts: BTreeMap<String, String>,
}
impl Lower {
    fn scoped(&mut self, body: &[Stmt<'_>], c: &mut Context) -> Result<()> {
        let outer = c.names.clone();
        let outer_checked = c.checked.clone();
        for s in body {
            self.statement(s, c)?;
        }
        c.names = outer;
        c.checked = outer_checked;
        Ok(())
    }
    fn function(
        &mut self,
        name: &str,
        params: &[Param<'_>],
        body: &[Stmt<'_>],
        asynchronous: bool,
        outer: &mut Context,
    ) -> Result<()> {
        if outer.loop_depth > 0 {
            return Err("closures inside loops are not supported by this VM slice".into());
        }
        let mut c = Context::new(name, asynchronous);
        let mut captures = vec![];
        for (name, slot) in &outer.names {
            c.bind(name);
            captures.push(*slot);
        }
        // Reads of cycle-checked imports stay checked inside closures, so a
        // function called while the cycle is still initializing errors by
        // name instead of reading the sentinel.
        c.checked = outer.checked.clone();
        c.function.captures = captures.len();
        c.function.parameters = params.len();
        for p in params {
            if p.default_value.is_some() {
                return Err("default parameters are unsupported".into());
            }
            c.bind(
                p.binding
                    .identifier()
                    .ok_or("tuple parameters are unsupported")?,
            );
        }
        let index = self.functions.len();
        self.functions.push(c.function.clone());
        for s in body {
            self.statement(s, &mut c)?;
        }
        c.emit(Op::Const(Literal::Unit));
        c.emit(Op::Return);
        self.functions[index] = c.function;
        outer.emit(Op::Closure {
            function: index,
            captures,
        });
        Ok(())
    }
    fn named_function(
        &mut self,
        name: &str,
        params: &[Param<'_>],
        types: &[TypeParam<'_>],
        body: &[Stmt<'_>],
        asynchronous: bool,
        c: &mut Context,
    ) -> Result<()> {
        if !types.is_empty() {
            return Err("generic functions unsupported in VM experiment".into());
        }
        let slot = c.bind(name);
        self.function(name, params, body, asynchronous, c)?;
        c.emit(Op::Store(slot));
        Ok(())
    }
    fn statement(&mut self, s: &Stmt<'_>, c: &mut Context) -> Result<()> {
        match s {
            Stmt::Import { .. } | Stmt::Empty { .. } => {}
            Stmt::Const { name, value, .. }
            | Stmt::Let { name, value, .. }
            | Stmt::Export {
                decl: ExportDecl::Const { name, value, .. },
                ..
            } => {
                self.expr(value, c)?;
                let slot = c.bind(name);
                c.emit(Op::Store(slot));
            }
            Stmt::Function {
                name,
                params,
                type_params,
                body,
                is_async,
                ..
            }
            | Stmt::Export {
                decl:
                    ExportDecl::Function {
                        name,
                        params,
                        type_params,
                        body,
                        is_async,
                        ..
                    },
                ..
            } => self.named_function(name, params, type_params, body, *is_async, c)?,
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    self.expr(v, c)?;
                } else {
                    c.emit(Op::Const(Literal::Unit));
                }
                c.emit(Op::Return);
            }
            Stmt::Expr { expr, .. } => {
                self.expr(expr, c)?;
                c.emit(Op::Pop);
            }
            Stmt::Block { body, .. } => self.scoped(body, c)?,
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.expr(condition, c)?;
                let other = c.emit(Op::JumpIfFalse(0));
                self.scoped(then_body, c)?;
                let end = c.emit(Op::Jump(0));
                c.patch(other);
                self.scoped(else_body, c)?;
                c.patch(end);
            }
            Stmt::For {
                init,
                condition,
                step,
                body,
                ..
            } => {
                let outer = c.names.clone();
                c.loop_depth += 1;
                if let Some(init) = init {
                    match init {
                        ForInit::Const { name, value } | ForInit::Let { name, value } => {
                            self.expr(value, c)?;
                            let slot = c.bind(name);
                            c.emit(Op::Store(slot));
                        }
                        ForInit::Expr(e) => {
                            self.expr(e, c)?;
                            c.emit(Op::Pop);
                        }
                    }
                }
                let start = c.function.code.len();
                if let Some(cond) = condition {
                    self.expr(cond, c)?;
                } else {
                    c.emit(Op::Const(Literal::Bool(true)));
                }
                let end = c.emit(Op::JumpIfFalse(0));
                self.scoped(body, c)?;
                if let Some(step) = step {
                    self.expr(step, c)?;
                    c.emit(Op::Pop);
                }
                c.emit(Op::Jump(start));
                c.patch(end);
                c.names = outer;
                c.loop_depth -= 1;
            }
            _ => {
                return Err(format!(
                    "{}:{}: statement is unsupported by VM experiment",
                    s.span().start.line,
                    s.span().start.column
                ));
            }
        }
        Ok(())
    }
    fn expr(&mut self, e: &Expr<'_>, c: &mut Context) -> Result<()> {
        match e {
            Expr::JsxElement { element, .. } => {
                if !matches!(
                    element.tag,
                    "view" | "div" | "p" | "span" | "button" | "input"
                ) {
                    return Err(format!("unsupported VM UI primitive: {}", element.tag));
                }
                c.emit(Op::Const(Literal::String(element.tag.into())));
                let mut names = vec!["tag".into()];
                for attr in element.attributes {
                    if !matches!(
                        attr.name,
                        "className" | "onClick" | "value" | "placeholder" | "onInput" | "onKeyDown"
                    ) {
                        return Err(format!("unsupported VM UI attribute: {}", attr.name));
                    }
                    let value = attr.value.as_ref().ok_or("UI attribute requires a value")?;
                    if matches!(attr.name, "onClick" | "onInput" | "onKeyDown")
                        && !matches!(
                            value,
                            Expr::Function {
                                is_async: false,
                                ..
                            }
                        )
                    {
                        return Err(
                            "VM UI click handlers must currently be synchronous function literals"
                                .into(),
                        );
                    }
                    if matches!(attr.name, "className" | "value" | "placeholder")
                        && !matches!(value, Expr::String { .. })
                    {
                        let body = [Stmt::Return {
                            value: Some(value.clone()),
                            span: value.span(),
                        }];
                        self.function("<ui attribute>", &[], &body, false, c)?;
                    } else {
                        self.expr(value, c)?;
                    }
                    names.push(attr.name.into());
                }
                let mut children = 0;
                for child in element.children {
                    match child {
                        Expr::JsxText { value, .. } => {
                            let mut text = value.split_whitespace().collect::<Vec<_>>().join(" ");
                            if text.is_empty() {
                                continue;
                            }
                            // Preserve inline separation around expressions, but not
                            // indentation from multiline markup.
                            if !value.contains('\n') {
                                if value.starts_with(char::is_whitespace) {
                                    text.insert(0, ' ');
                                }
                                if value.ends_with(char::is_whitespace) {
                                    text.push(' ');
                                }
                            }
                            c.emit(Op::Const(Literal::String(text)));
                        }
                        Expr::JsxElement { .. } => self.expr(child, c)?,
                        _ => {
                            let body = [Stmt::Return {
                                value: Some(child.clone()),
                                span: child.span(),
                            }];
                            self.function("<ui binding>", &[], &body, false, c)?;
                        }
                    }
                    children += 1;
                }
                c.emit(Op::List(children));
                names.push("children".into());
                c.emit(Op::Record(names));
            }
            Expr::Number { value, .. } => {
                c.emit(Op::Const(Literal::Number(*value)));
            }
            Expr::String { value, .. } => {
                c.emit(Op::Const(Literal::String((*value).into())));
            }
            Expr::Boolean { value, .. } => {
                c.emit(Op::Const(Literal::Bool(*value)));
            }
            Expr::None { .. } => {
                c.emit(Op::Const(Literal::Unit));
            }
            Expr::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expr(condition, c)?;
                let otherwise = c.emit(Op::JumpIfFalse(0));
                self.expr(then_branch, c)?;
                let end = c.emit(Op::Jump(0));
                c.patch(otherwise);
                self.expr(else_branch, c)?;
                c.patch(end);
            }
            Expr::Identifier { name, .. } => {
                c.emit_load(name)?;
            }
            Expr::Paren { expr, .. } => self.expr(expr, c)?,
            Expr::Binary {
                left, op, right, ..
            } => {
                if matches!(
                    op,
                    BinOp::Assign | BinOp::AddAssign | BinOp::SubAssign | BinOp::MulAssign
                ) {
                    let Expr::Identifier { name, .. } = left else {
                        return Err(
                            "only binding assignment is supported; collections are immutable"
                                .into(),
                        );
                    };
                    let slot = c.slot(name)?;
                    if *op != BinOp::Assign {
                        c.emit_load(name)?;
                    }
                    self.expr(right, c)?;
                    match op {
                        BinOp::AddAssign => {
                            c.emit(Op::Add);
                        }
                        BinOp::SubAssign => {
                            c.emit(Op::Sub);
                        }
                        BinOp::MulAssign => {
                            c.emit(Op::Mul);
                        }
                        _ => {}
                    }
                    c.emit(Op::Dup);
                    c.emit(Op::Store(slot));
                } else {
                    self.expr(left, c)?;
                    self.expr(right, c)?;
                    c.emit(match op {
                        BinOp::Add => Op::Add,
                        BinOp::Sub => Op::Sub,
                        BinOp::Mul => Op::Mul,
                        BinOp::Div => Op::Div,
                        BinOp::Lt => Op::Less,
                        BinOp::Eq => Op::Equal,
                        _ => return Err(format!("operator {op:?} unsupported")),
                    });
                }
            }
            Expr::Call {
                callee,
                args,
                type_args,
                ..
            } => {
                if !type_args.is_empty() {
                    return Err("explicit type arguments unsupported".into());
                }
                if let Expr::FieldAccess {
                    object,
                    field: "has",
                    ..
                } = callee
                {
                    let [index] = *args else {
                        return Err("has requires one index".into());
                    };
                    self.expr(object, c)?;
                    self.expr(index, c)?;
                    c.emit(Op::ListHas);
                    return Ok(());
                }
                if let Expr::FieldAccess {
                    object,
                    field: "map",
                    ..
                } = callee
                {
                    let [mapper] = *args else {
                        return Err("map requires one callback".into());
                    };
                    let Expr::Function {
                        params,
                        is_async: false,
                        ..
                    } = mapper
                    else {
                        return Err("map requires a synchronous function literal".into());
                    };
                    if !(1..=2).contains(&params.len()) {
                        return Err("map callback takes value and optional index".into());
                    }
                    // Evaluate receiver/callback once; each Call allocates fresh parameter cells.
                    let array = c.bind(&format!("<map array {}>", c.function.locals));
                    let callback = c.bind(&format!("<map callback {}>", c.function.locals));
                    let result = c.bind(&format!("<map result {}>", c.function.locals));
                    let index = c.bind(&format!("<map index {}>", c.function.locals));
                    self.expr(object, c)?;
                    c.emit(Op::Store(array));
                    self.expr(mapper, c)?;
                    c.emit(Op::Store(callback));
                    c.emit(Op::List(0));
                    c.emit(Op::Store(result));
                    c.emit(Op::Const(Literal::Number(0.)));
                    c.emit(Op::Store(index));
                    let start = c.function.code.len();
                    c.emit(Op::Load(index));
                    c.emit(Op::Load(array));
                    c.emit(Op::Field("length".into()));
                    c.emit(Op::Less);
                    let end = c.emit(Op::JumpIfFalse(0));
                    c.emit(Op::Load(result));
                    c.emit(Op::Load(callback));
                    c.emit(Op::Load(array));
                    c.emit(Op::Load(index));
                    c.emit(Op::Index);
                    if params.len() == 2 {
                        c.emit(Op::Load(index));
                    }
                    c.emit(Op::Call(params.len()));
                    c.emit(Op::ListAppend);
                    c.emit(Op::Store(result));
                    c.emit(Op::Load(index));
                    c.emit(Op::Const(Literal::Number(1.)));
                    c.emit(Op::Add);
                    c.emit(Op::Store(index));
                    c.emit(Op::Jump(start));
                    c.patch(end);
                    c.emit(Op::Load(result));
                    return Ok(());
                }
                let host = if let Expr::Identifier { name, .. } = callee {
                    if !c.names.contains_key(*name) {
                        self.hosts.get(*name).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(operation) = host {
                    for arg in *args {
                        self.expr(arg, c)?;
                    }
                    c.emit(Op::Host {
                        operation,
                        arguments: args.len(),
                    });
                } else {
                    self.expr(callee, c)?;
                    for arg in *args {
                        self.expr(arg, c)?;
                    }
                    c.emit(Op::Call(args.len()));
                }
            }
            Expr::Function {
                params,
                body,
                is_async,
                ..
            } => self.function("<closure>", params, body, *is_async, c)?,
            Expr::Await { expr, .. } => {
                self.expr(expr, c)?;
                c.emit(Op::Await);
            }
            Expr::Array { elements, .. } => {
                for e in *elements {
                    self.expr(e, c)?;
                }
                c.emit(Op::List(elements.len()));
            }
            Expr::Object { fields, .. } => {
                let mut names = vec![];
                for f in *fields {
                    self.expr(&f.value, c)?;
                    names.push(f.key.to_string());
                }
                c.emit(Op::Record(names));
            }
            Expr::FieldAccess { object, field, .. } => {
                self.expr(object, c)?;
                c.emit(Op::Field((*field).into()));
            }
            Expr::IndexAccess { object, index, .. } => {
                self.expr(object, c)?;
                self.expr(index, c)?;
                c.emit(Op::Index);
            }
            _ => {
                return Err(format!(
                    "{}:{}: expression is unsupported by VM experiment",
                    e.span().start.line,
                    e.span().start.column
                ));
            }
        }
        Ok(())
    }
}
