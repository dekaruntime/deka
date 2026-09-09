//! deka#750 — module-preserving client treeshake.
//!
//! `prune_unreferenced_exports` drops top-level function declarations that are
//! unreachable from the exports a module's importers actually use, plus
//! everything only those functions referenced. It answers: "the browser is
//! shipped the SSR-producer half of `ui/island-marker`" without splitting the
//! single-source module (deka#622 finding C): `formatIslandStart`,
//! `formatIslandEnd`, `encodeB64`, and `utf8Bytes` are only reachable from the
//! producer exports, so a client graph that imports only `parseIslandMarker`
//! sheds all four.
//!
//! The pass is deliberately conservative:
//! - only plain top-level `function` declarations are removal candidates
//!   (they bind without side effects); imports, variables, classes, default
//!   exports, and statements stay,
//! - binding/use pairs are distinguished with the SWC resolver's hygiene
//!   marks, so shadowed locals never keep a same-named top-level function
//!   alive and never get dropped as a false match,
//! - a module using direct `eval` or `with` is returned unchanged (dynamic
//!   scope can observe any binding),
//! - callers must pass the complete importer set; anything that imports a
//!   module without naming its bindings (namespace/default/side-effect import,
//!   `export *`, dynamic `import()`) is surfaced by [`scan_module_imports`] so
//!   the caller keeps every export of that module.
//!
//! Like [`crate::optimize_emitted_module`], source and output are ESM and
//! relative specifiers are preserved.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use swc_common::{FileName, GLOBALS, Globals, Mark, SourceMap, Span, sync::Lrc};
use swc_ecma_ast::*;
use swc_ecma_codegen::{Emitter, text_writer::JsWriter};
use swc_ecma_parser::{EsSyntax, Parser, StringInput, Syntax, lexer::Lexer};
use swc_ecma_transforms_base::resolver;
use swc_ecma_visit::{Visit, VisitWith};

/// How a module's bindings are imported, per specifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportUse {
    /// `import { a, b } from "..."` / `export { a, b } from "..."`: only these
    /// named bindings may be observed.
    Named(HashSet<String>),
    /// Namespace, default-only, or side-effect import, `export *`, or dynamic
    /// `import()`: any binding may be observed; keep them all.
    KeepAll,
}

/// Every import of a module, for cross-module export pruning.
#[derive(Debug, Default)]
pub struct ModuleScan {
    pub imports: Vec<(String, ImportUse)>,
    /// True when the module uses direct `eval` or `with`; pruning such a
    /// module is unsound, so the caller should skip it.
    pub uses_dynamic_scope: bool,
}

struct ImportCollector {
    imports: Vec<(String, ImportUse)>,
    uses_dynamic_scope: bool,
}

impl Visit for ImportCollector {
    fn visit_import_decl(&mut self, node: &ImportDecl) {
        self.imports.push((node.src.value.to_atom_lossy().to_string(), import_use(&node.specifiers)));
    }

    fn visit_named_export(&mut self, node: &NamedExport) {
        if let Some(src) = &node.src {
            let mut names = HashSet::new();
            let mut all_named = !node.specifiers.is_empty();
            for spec in &node.specifiers {
                match spec {
                    ExportSpecifier::Named(named) => {
                        // `export { a as b } from`: the local side is `orig`.
                        names.insert(module_export_name(&named.orig));
                    }
                    ExportSpecifier::Default(_) | ExportSpecifier::Namespace(_) => {
                        all_named = false;
                    }
                }
            }
            if all_named {
                self.imports
                    .push((src.value.to_atom_lossy().to_string(), ImportUse::Named(names)));
            } else {
                self.imports.push((src.value.to_atom_lossy().to_string(), ImportUse::KeepAll));
            }
        }
        node.visit_children_with(self);
    }

    fn visit_export_all(&mut self, node: &ExportAll) {
        self.imports
            .push((node.src.value.to_atom_lossy().to_string(), ImportUse::KeepAll));
    }

    fn visit_call_expr(&mut self, node: &CallExpr) {
        if let Callee::Import(_) = node.callee {
            if let Some(arg) = node.args.first() {
                if let Expr::Lit(Lit::Str(str_)) = &*arg.expr {
                    self.imports
                        .push((str_.value.to_atom_lossy().to_string(), ImportUse::KeepAll));
                }
            }
        }
        if let Callee::Expr(expr) = &node.callee {
            if let Expr::Ident(ident) = &**expr {
                if ident.sym == *"eval" {
                    self.uses_dynamic_scope = true;
                }
            }
        }
        node.visit_children_with(self);
    }

    fn visit_with_stmt(&mut self, node: &WithStmt) {
        self.uses_dynamic_scope = true;
        node.visit_children_with(self);
    }
}

fn import_use(specifiers: &[ImportSpecifier]) -> ImportUse {
    // A side-effect import (`import "x"`) names no bindings; anything the
    // module binds may be observed through it.
    if specifiers.is_empty() {
        return ImportUse::KeepAll;
    }
    let mut names = HashSet::new();
    for spec in specifiers {
        match spec {
            ImportSpecifier::Named(named) => {
                names.insert(named.local.sym.to_string());
            }
            ImportSpecifier::Default(_) | ImportSpecifier::Namespace(_) => {
                return ImportUse::KeepAll;
            }
        }
    }
    ImportUse::Named(names)
}

fn module_export_name(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::Ident(ident) => ident.sym.to_string(),
        ModuleExportName::Str(str_) => str_.value.to_atom_lossy().to_string(),
    }
}

/// Scan a module's imports. `path` labels parse errors.
pub fn scan_module_imports(source: &str, path: &Path) -> Result<ModuleScan, String> {
    let module = parse_module(source, path)?;
    let mut collector = ImportCollector {
        imports: Vec::new(),
        uses_dynamic_scope: false,
    };
    module.visit_with(&mut collector);
    Ok(ModuleScan {
        imports: collector.imports,
        uses_dynamic_scope: collector.uses_dynamic_scope,
    })
}

fn parse_module(source: &str, path: &Path) -> Result<Module, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(FileName::Real(path.to_path_buf()).into(), source.to_string());
    let syntax = Syntax::Es(EsSyntax {
        jsx: false,
        export_default_from: true,
        import_attributes: true,
        ..Default::default()
    });
    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    parser
        .parse_module()
        .map_err(|err| format!("failed to parse {}: {err:?}", path.display()))
}

fn emit_module(module: &Module, cm: Lrc<SourceMap>) -> Result<String, String> {
    let mut buf = Vec::new();
    let mut emitter = Emitter {
        cfg: swc_ecma_codegen::Config::default(),
        comments: None,
        cm: cm.clone(),
        wr: JsWriter::new(cm, "\n", &mut buf, None),
    };
    emitter
        .emit_module(module)
        .map_err(|err| format!("failed to emit pruned JavaScript: {err}"))?;
    String::from_utf8(buf).map_err(|err| format!("pruned JavaScript was not UTF-8: {err}"))
}

/// A module-scope binding occurrence, keyed by name and hygiene context (the
/// resolver gives shadowed inner bindings a different context, which is what
/// keeps the use/binding split sound).
type BindingId = (String, swc_common::SyntaxContext);

/// Collect every `Ident` occurrence (binding sites included) below `node`.
struct OccurrenceCollector {
    ids: HashSet<BindingId>,
}

impl Visit for OccurrenceCollector {
    fn visit_ident(&mut self, node: &Ident) {
        self.ids
            .insert((node.sym.to_string(), node.ctxt));
    }
}

/// Collect the idents bound by a pattern (object/array/rest nests included).
fn pattern_bindings(pat: &Pat, out: &mut HashSet<BindingId>) {
    match pat {
        Pat::Ident(ident) => {
            out.insert((ident.id.sym.to_string(), ident.id.ctxt));
        }
        Pat::Array(array) => {
            for elem in array.elems.iter().flatten() {
                pattern_bindings(elem, out);
            }
        }
        Pat::Object(object) => {
            for prop in &object.props {
                match prop {
                    ObjectPatProp::KeyValue(kv) => pattern_bindings(&kv.value, out),
                    ObjectPatProp::Assign(assign) => {
                        // `IdentName` carries no hygiene context; an empty one
                        // is fine here because the declaring item is a variable
                        // declaration (always a root) anyway.
                        out.insert((
                            assign.key.sym.to_string(),
                            swc_common::SyntaxContext::empty(),
                        ));
                    }
                    ObjectPatProp::Rest(rest) => pattern_bindings(&rest.arg, out),
                }
            }
        }
        Pat::Rest(rest) => pattern_bindings(&rest.arg, out),
        Pat::Assign(assign) => pattern_bindings(&assign.left, out),
        Pat::Invalid(_) | Pat::Expr(_) => {}
    }
}

struct FnCandidate {
    item: usize,
    exported_names: Vec<String>,
}

/// Drop top-level function declarations unreachable from `keep_exports`
/// (the exported names this module's importers use), transitively through the
/// module's own bindings. See the module doc for the soundness envelope.
pub fn prune_unreferenced_exports(
    source: &str,
    path: &Path,
    keep_exports: &HashSet<String>,
) -> Result<String, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        FileName::Real(path.to_path_buf()).into(),
        source.to_string(),
    );
    let syntax = Syntax::Es(EsSyntax {
        jsx: false,
        export_default_from: true,
        import_attributes: true,
        ..Default::default()
    });
    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let mut module = parser
        .parse_module()
        .map_err(|err| format!("failed to parse {}: {err:?}", path.display()))?;

    if has_dynamic_scope(&module) {
        return Ok(source.to_string());
    }

    let globals = Globals::new();
    let shebang = module.shebang.take();
    module = GLOBALS.set(&globals, || {
        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();
        let mut program = Program::Module(module);
        resolver(unresolved_mark, top_level_mark, false).process(&mut program);
        match program {
            Program::Module(module) => module,
            Program::Script(_) => unreachable!("prune input parsed as a module"),
        }
    });

    // Module-scope bindings: name -> declaring item index.
    let mut binding_item: HashMap<BindingId, usize> = HashMap::new();
    // Per-item binding occurrences, to subtract from all occurrences.
    let mut item_bindings: Vec<HashSet<BindingId>> = vec![HashSet::new(); module.body.len()];
    // Per-item uses of module-scope bindings.
    let mut item_refs: Vec<HashSet<usize>> = vec![HashSet::new(); module.body.len()];
    let mut candidates: Vec<FnCandidate> = Vec::new();
    let mut has_keep_all_export = false;

    for (index, item) in module.body.iter().enumerate() {
        let mut bindings = HashSet::new();
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                for spec in &import.specifiers {
                    let local = match spec {
                        ImportSpecifier::Named(named) => &named.local,
                        ImportSpecifier::Default(default) => &default.local,
                        ImportSpecifier::Namespace(namespace) => &namespace.local,
                    };
                    bindings.insert((local.sym.to_string(), local.ctxt));
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                collect_decl_bindings(&export.decl, &mut bindings, &mut candidates, index, true);
            }
            ModuleItem::Stmt(Stmt::Decl(decl)) => {
                collect_decl_bindings(decl, &mut bindings, &mut candidates, index, false);
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => {
                match &export.decl {
                    DefaultDecl::Fn(fn_expr) => {
                        if let Some(ident) = &fn_expr.ident {
                            bindings.insert((ident.sym.to_string(), ident.ctxt));
                        }
                    }
                    DefaultDecl::Class(class_expr) => {
                        if let Some(ident) = &class_expr.ident {
                            bindings.insert((ident.sym.to_string(), ident.ctxt));
                        }
                    }
                    DefaultDecl::TsInterfaceDecl(_) => {}
                }
                has_keep_all_export = true;
            }
            ModuleItem::ModuleDecl(
                ModuleDecl::ExportDefaultExpr(_) | ModuleDecl::ExportAll(_),
            ) => has_keep_all_export = true,
            _ => {}
        }
        for id in &bindings {
            binding_item.insert(id.clone(), index);
        }
        item_bindings[index] = bindings;
    }

    // Occurrences per item, minus that item's own binding sites, give uses.
    for (index, item) in module.body.iter().enumerate() {
        let mut collector = OccurrenceCollector {
            ids: HashSet::new(),
        };
        item.visit_with(&mut collector);
        let uses: HashSet<BindingId> = collector.ids.difference(&item_bindings[index]).cloned().collect();
        for id in uses {
            if let Some(&target) = binding_item.get(&id) {
                item_refs[index].insert(target);
            }
        }
    }

    // Roots: everything that is not a droppable plain function declaration,
    // plus candidates exporting a kept name. Default exports make every
    // binding observable, so nothing may be dropped.
    let mut reachable: HashSet<usize> = HashSet::new();
    let mut worklist: Vec<usize> = Vec::new();
    if !has_keep_all_export {
        for (index, _) in module.body.iter().enumerate() {
            let droppable = candidates.iter().any(|c| c.item == index);
            let kept = candidates.iter().any(|c| {
                c.item == index && c.exported_names.iter().any(|name| keep_exports.contains(name))
            });
            if !droppable || kept {
                if reachable.insert(index) {
                    worklist.push(index);
                }
            }
        }
    } else {
        for index in 0..module.body.len() {
            reachable.insert(index);
        }
    }

    while let Some(index) = worklist.pop() {
        for &next in &item_refs[index] {
            if reachable.insert(next) {
                worklist.push(next);
            }
        }
    }

    let dropped: HashSet<usize> = candidates
        .iter()
        .map(|c| c.item)
        .filter(|index| !reachable.contains(index))
        .collect();
    if dropped.is_empty() {
        return Ok(source.to_string());
    }

    let mut dropped_ids: HashSet<BindingId> = HashSet::new();
    for index in &dropped {
        for (id, item) in &binding_item {
            if item == index {
                dropped_ids.insert(id.clone());
            }
        }
    }

    // Rebuild the body without dropped functions; export lists referencing a
    // dropped binding lose that specifier (an empty list goes away entirely).
    let mut body: Vec<ModuleItem> = Vec::with_capacity(module.body.len());
    for (index, item) in module.body.into_iter().enumerate() {
        if dropped.contains(&index) {
            continue;
        }
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(mut named)) if named.src.is_none() => {
                named.specifiers.retain(|spec| match spec {
                    ExportSpecifier::Named(named) => {
                        let local = match &named.orig {
                            ModuleExportName::Ident(ident) => {
                                (ident.sym.to_string(), ident.ctxt)
                            }
                            ModuleExportName::Str(str_) => (
                                str_.value.to_atom_lossy().to_string(),
                                swc_common::SyntaxContext::empty(),
                            ),
                        };
                        !dropped_ids.contains(&local)
                    }
                    _ => true,
                });
                if !named.specifiers.is_empty() {
                    body.push(ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(named)));
                }
            }
            other => body.push(other),
        }
    }
    let pruned = Module {
        span: Span::default(),
        body,
        shebang,
    };
    emit_module(&pruned, cm)
}

fn has_dynamic_scope(module: &Module) -> bool {
    let mut collector = ImportCollector {
        imports: Vec::new(),
        uses_dynamic_scope: false,
    };
    module.visit_with(&mut collector);
    collector.uses_dynamic_scope
}

fn collect_decl_bindings(
    decl: &Decl,
    bindings: &mut HashSet<BindingId>,
    candidates: &mut Vec<FnCandidate>,
    item: usize,
    exported: bool,
) {
    match decl {
        Decl::Fn(fn_decl) => {
            bindings.insert((fn_decl.ident.sym.to_string(), fn_decl.ident.ctxt));
            if exported {
                candidates.push(FnCandidate {
                    item,
                    exported_names: vec![fn_decl.ident.sym.to_string()],
                });
            } else {
                candidates.push(FnCandidate {
                    item,
                    exported_names: Vec::new(),
                });
            }
        }
        Decl::Class(class) => {
            bindings.insert((class.ident.sym.to_string(), class.ident.ctxt));
        }
        Decl::Var(var) => {
            for declarator in &var.decls {
                pattern_bindings(&declarator.name, bindings);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("test.js")
    }

    #[test]
    fn prune_drops_ssr_producer_half_of_island_marker() {
        let keep: HashSet<String> = HashSet::from(["parseIslandMarker".to_string()]);
        let out = prune_unreferenced_exports(deka_ui::ISLAND_MARKER, &path(), &keep)
            .expect("prune island-marker");
        for symbol in ["formatIslandStart", "formatIslandEnd", "encodeB64", "utf8Bytes"] {
            assert!(!out.contains(symbol), "{symbol} must be pruned:\n{out}");
        }
        for symbol in ["parseIslandMarker", "decodeB64", "ISLAND_MARKER_FIELDS"] {
            assert!(out.contains(symbol), "{symbol} must survive:\n{out}");
        }
        // The consumer half must still parse as a module and export the entry
        // point importers use.
        assert!(out.contains("export"), "kept exports must remain:\n{out}");
        assert!(out.contains("export function parseIslandMarker"), "{out}");
    }

    #[test]
    fn prune_keeps_transitively_referenced_private_functions() {
        let source = "export function used() { return helper(); }\n\
                      function helper() { return 1; }\n\
                      export function unused() { return 2; }\n";
        let keep: HashSet<String> = HashSet::from(["used".to_string()]);
        let out = prune_unreferenced_exports(source, &path(), &keep).expect("prune");
        assert!(out.contains("function used"), "{out}");
        assert!(out.contains("function helper"), "helper is reachable from used:\n{out}");
        assert!(!out.contains("unused"), "{out}");
    }

    #[test]
    fn prune_drops_unkept_unreachable_exports() {
        // The keep set is the complete importer contract: an exported name no
        // importer uses and nothing reachable references is dead weight.
        let source = "export function api() { return internal(); }\n\
                      function internal() { return 1; }\n\
                      export function live() { return internal2(); }\n\
                      function internal2() { return 2; }\n";
        let keep: HashSet<String> = HashSet::from(["live".to_string()]);
        let out = prune_unreferenced_exports(source, &path(), &keep).expect("prune");
        assert!(!out.contains("function api("), "{out}");
        assert!(!out.contains("function internal("), "{out}");
        assert!(out.contains("function live("), "{out}");
        assert!(out.contains("function internal2("), "{out}");
    }

    #[test]
    fn prune_is_noop_when_nothing_is_droppable() {
        let source = "export const a = 1;\nexport function f() { return a; }\n";
        let keep: HashSet<String> = HashSet::from(["f".to_string()]);
        let out = prune_unreferenced_exports(source, &path(), &keep).expect("prune");
        assert_eq!(out, source, "nothing removable -> byte-identical");
    }

    #[test]
    fn prune_noop_on_eval_and_default_exports() {
        let with_eval = "function f() { return 1; }\nexport function g() { return eval('1'); }\n";
        let keep: HashSet<String> = HashSet::from(["g".to_string()]);
        let out = prune_unreferenced_exports(with_eval, &path(), &keep).expect("prune");
        assert!(out.contains("function f"), "eval pins every binding:\n{out}");

        let with_default =
            "function f() { return 1; }\nexport default function main() { return f(); }\n";
        let out = prune_unreferenced_exports(with_default, &path(), &HashSet::new())
            .expect("prune");
        assert!(out.contains("function f"), "default export pins every binding:\n{out}");
    }

    #[test]
    fn prune_respects_shadowing() {
        let source = "function f() { return 1; }\n\
                      export function g() { const f = 2; return f; }\n";
        let keep: HashSet<String> = HashSet::from(["g".to_string()]);
        let out = prune_unreferenced_exports(source, &path(), &keep).expect("prune");
        assert!(
            !out.contains("function f()"),
            "shadowed local must not keep the top-level f alive:\n{out}"
        );
    }

    #[test]
    fn scan_distinguishes_named_namespace_side_effect_and_dynamic() {
        let source = "import { a, b } from \"ui/jsx\";\n\
                      import * as ns from \"ui/router\";\n\
                      import \"ui/suspense\";\n\
                      import def from \"ui/form\";\n\
                      const m = await import(\"ui/reactive\");\n\
                      export { x } from \"ui/server\";\n";
        let scan = scan_module_imports(source, &path()).expect("scan");
        let use_of = |spec: &str| {
            scan.imports
                .iter()
                .find(|(s, _)| s == spec)
                .map(|(_, kind)| kind.clone())
        };
        assert_eq!(
            use_of("ui/jsx"),
            Some(ImportUse::Named(HashSet::from(["a".to_string(), "b".to_string()])))
        );
        assert_eq!(use_of("ui/router"), Some(ImportUse::KeepAll));
        assert_eq!(use_of("ui/suspense"), Some(ImportUse::KeepAll));
        assert_eq!(use_of("ui/form"), Some(ImportUse::KeepAll));
        assert_eq!(use_of("ui/reactive"), Some(ImportUse::KeepAll));
        assert_eq!(
            use_of("ui/server"),
            Some(ImportUse::Named(HashSet::from(["x".to_string()])))
        );
        assert!(!scan.uses_dynamic_scope);
    }

    #[test]
    fn scan_flags_eval() {
        let scan = scan_module_imports("export function g() { return eval('1'); }\n", &path())
            .expect("scan");
        assert!(scan.uses_dynamic_scope);
    }
}
