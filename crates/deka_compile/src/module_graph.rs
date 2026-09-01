//! Cross-file module graph compilation for DekaScript compiler v2.
//!
//! This module discovers all reachable `.ds` files from an entry point,
//! resolves relative and bare stdlib specifiers, and compiles each module
//! through the v2 pipeline.  Exported structs, enums, type aliases, and
//! receiver methods are propagated through the module graph so importers can
//! construct imported structs and match imported enums with full typechecking.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use bumpalo::Bump;
use deka_syntax::Diagnostic;

use crate::{
    compile_to_js_with_imports_and_options, parse_source_module_meta, CompileOptions,
};
use crate::shake::{self, ShakeModule, ShakePlan};

/// Compiler-provided JS runtime (`ui/jsx`, `ui/form`, …). These are not
/// DekaScript modules: hosts materialize the files, and the graph leaves the
/// import specifier intact.
fn is_compiler_ui_spec(spec: &str) -> bool {
    let bare = spec.trim().strip_prefix("@deka/").unwrap_or(spec.trim());
    bare == "ui" || bare.starts_with("ui/")
}

/// A module loader supplies source text and resolves specifiers for the
/// graph compiler.
///
/// Callers provide the loader so the compiler can run against a filesystem,
/// an in-memory test fixture set, or a virtual project layout.
pub trait ModuleLoader {
    /// Resolve a module specifier relative to the importing file.
    ///
    /// Returns the absolute path to the DekaScript source file that should
    /// be loaded.
    fn resolve(&self, specifier: &str, referrer: &Path) -> Result<PathBuf, String>;

    /// Read the source text for a resolved module path.
    fn load(&self, path: &Path) -> Result<String, String>;
}

/// Filesystem loader that mirrors the runtime's module resolution rules.
///
/// Resolution order (kept in sync with `runtime_core::module_spec`):
///
/// 1. `@/path` → project root.
/// 2. `/abs/path` → absolute path (must still lie inside the project root).
/// 3. `./path` or `../path` → relative to the importing file's directory.
/// 4. Bare specifier (e.g. `json`, `@deka/crypto`) → a project-local link
///    from `.deka/links.json`, then `ds_modules/` (with `@deka/` aliases),
///    falling back to `module_root` when provided.
pub struct FsModuleLoader {
    project_root: PathBuf,
    module_root: Option<PathBuf>,
    linked_modules: BTreeMap<String, PathBuf>,
    link_error: Option<String>,
}

impl FsModuleLoader {
    pub fn new(project_root: PathBuf) -> Self {
        let (linked_modules, link_error) = load_linked_modules(&project_root);
        Self {
            project_root,
            module_root: None,
            linked_modules,
            link_error,
        }
    }

    /// Create a loader with an explicit module root for resolving bare stdlib
    /// imports. When a bare specifier cannot be found under the project's
    /// `ds_modules/`, the loader tries `<module_root>/ds_modules/` before
    /// giving up.
    pub fn with_module_root(project_root: PathBuf, module_root: PathBuf) -> Self {
        let (linked_modules, link_error) = load_linked_modules(&project_root);
        Self {
            project_root,
            module_root: Some(module_root),
            linked_modules,
            link_error,
        }
    }

    fn resolve_ds_file(&self, base: &Path) -> Option<PathBuf> {
        runtime_core::module_spec::resolve_ds_source_file(base)
    }

    fn guard_project_root(&self, path: &Path) -> Result<PathBuf, String> {
        let canon_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let canon_root = std::fs::canonicalize(&self.project_root)
            .unwrap_or_else(|_| self.project_root.clone());
        if canon_path.starts_with(&canon_root) {
            return Ok(canon_path);
        }
        // Files inside a linked package root are part of the project even
        // though they live outside the project directory on disk.
        for root in self.linked_modules.values() {
            let canon_link = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            if canon_path.starts_with(&canon_link) {
                return Ok(canon_path);
            }
        }
        Err(format!(
            "import escapes project root: {} is outside {}",
            canon_path.display(),
            canon_root.display()
        ))
    }

    fn resolve_linked_module(&self, specifier: &str) -> Option<PathBuf> {
        for (package, root) in &self.linked_modules {
            for alias in runtime_core::module_spec::module_spec_aliases(package) {
                let suffix = if specifier == alias {
                    ""
                } else if let Some(suffix) = specifier.strip_prefix(&(alias + "/")) {
                    suffix
                } else {
                    continue;
                };
                let base = if suffix.is_empty() {
                    root.clone()
                } else {
                    root.join(suffix)
                };
                if let Some(resolved) = self.resolve_ds_file(&base) {
                    let canonical = std::fs::canonicalize(&resolved).ok()?;
                    let canonical_root = std::fs::canonicalize(root).ok()?;
                    if canonical.starts_with(canonical_root) {
                        return Some(canonical);
                    }
                }
            }
        }
        None
    }
}

fn load_linked_modules(project_root: &Path) -> (BTreeMap<String, PathBuf>, Option<String>) {
    match runtime_core::modules::read_linked_modules(project_root) {
        Ok(links) => (links, None),
        Err(error) => (BTreeMap::new(), Some(error)),
    }
}

impl ModuleLoader for FsModuleLoader {
    fn resolve(&self, specifier: &str, referrer: &Path) -> Result<PathBuf, String> {
        if let Some(error) = &self.link_error {
            return Err(error.clone());
        }
        let trimmed = specifier.trim();

        if trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("file://")
        {
            return Err(format!(
                "unsupported module specifier '{}' (only relative and bare stdlib imports are supported)",
                trimmed
            ));
        }

        // Project-root alias.
        if let Some(rel) = trimmed.strip_prefix("@/") {
            if rel.split('/').any(|seg| seg == ".." || seg == ".") {
                return Err(format!(
                    "invalid project alias '{}': path cannot contain '.' or '..'",
                    trimmed
                ));
            }
            let base = self.project_root.join(rel);
            let resolved = self
                .resolve_ds_file(&base)
                .ok_or_else(|| format!("cannot resolve project alias '{}'", trimmed))?;
            return self.guard_project_root(&resolved);
        }

        // Absolute path.
        if trimmed.starts_with('/') {
            let base = PathBuf::from(trimmed);
            let resolved = self
                .resolve_ds_file(&base)
                .ok_or_else(|| format!("cannot resolve absolute import '{}'", trimmed))?;
            return self.guard_project_root(&resolved);
        }

        // Relative path.
        if trimmed.starts_with("./") || trimmed.starts_with("../") {
            let base = referrer
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(trimmed);
            let resolved = self
                .resolve_ds_file(&base)
                .ok_or_else(|| format!("cannot resolve relative import '{}'", trimmed))?;
            return self.guard_project_root(&resolved);
        }

        if let Some(resolved) = self.resolve_linked_module(trimmed) {
            return Ok(resolved);
        }

        // Bare / stdlib specifier.
        let modules_dir = runtime_core::modules::resolve_modules_dir(&self.project_root);
        let mut aliases = runtime_core::module_spec::module_spec_aliases(trimmed);
        if trimmed.contains('/') && !trimmed.starts_with('@') {
            aliases.push(format!("@deka/{}", trimmed));
        }
        for alias in &aliases {
            let base = modules_dir.join(alias);
            if let Some(resolved) = self.resolve_ds_file(&base) {
                return Ok(resolved);
            }
        }

        // Explicit module_root fallback for stdlib-only tenants.
        if let Some(module_root) = &self.module_root {
            let modules_dir = runtime_core::modules::resolve_modules_dir(module_root);
            for alias in &aliases {
                let base = modules_dir.join(alias);
                if let Some(resolved) = self.resolve_ds_file(&base) {
                    return Ok(resolved);
                }
            }
        }

        Err(format!(
            "cannot resolve module specifier '{}' (tried {:?})",
            trimmed, aliases
        ))
    }

    fn load(&self, path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))
    }
}

/// Options for compiling a module graph.
#[derive(Debug, Default, Clone)]
pub struct GraphCompileOptions {
    /// When true, any import-graph path to `ui/server` is a compile error.
    pub client: bool,
}

/// A discovered module and its outgoing dependencies.
#[derive(Debug)]
struct GraphModule {
    path: PathBuf,
    source: String,
    /// Resolved dependency path for each import specifier in this module.
    dependencies: HashMap<String, PathBuf>,
    /// Compiler-provided specifiers (`ui/jsx`, …) that are not `.ds` files.
    virtual_imports: Vec<String>,
}

/// Result of compiling a module graph.
#[derive(Debug)]
pub struct ModuleGraphResult {
    /// Absolute path of the entry module.
    pub entry: PathBuf,
    /// Map from absolute module path to emitted JavaScript.
    pub modules: HashMap<PathBuf, String>,
    /// Every import specifier written anywhere in the graph, as written.
    ///
    /// Callers that gate on dependencies need the transitive set, not the
    /// entry file's imports: a module the entry never mentions can pull in a
    /// stdlib package the project does not declare.  gated on
    /// entry-only imports and missed exactly that (deka#430).
    pub imports: BTreeSet<String>,
}

/// Compile every reachable `.ds` module from `entry` and return the emitted
/// JavaScript for each file.
///
/// The graph is discovered via the supplied loader, cycles are rejected with
/// diagnostics, and each module is compiled through the full v2 pipeline
/// (parse → typecheck → emit).  Type information is propagated from
/// dependencies to importers in topological order so cross-module structs,
/// enums, and receiver methods resolve correctly.
pub fn compile_module_graph(
    entry: &Path,
    loader: &dyn ModuleLoader,
) -> Result<ModuleGraphResult, Vec<Diagnostic>> {
    compile_module_graph_with_options(entry, loader, GraphCompileOptions::default())
}

/// Compile every reachable `.ds` module from `entry` with shaking options.
pub fn compile_module_graph_with_options(
    entry: &Path,
    loader: &dyn ModuleLoader,
    options: GraphCompileOptions,
) -> Result<ModuleGraphResult, Vec<Diagnostic>> {
    let entry = std::fs::canonicalize(entry).unwrap_or_else(|_| entry.to_path_buf());
    let arena = Bump::new();

    let mut modules: HashMap<PathBuf, GraphModule> = HashMap::new();
    let mut errors: Vec<Diagnostic> = Vec::new();
    let mut all_imports: BTreeSet<String> = BTreeSet::new();

    // ------------------------------------------------------------------
    // Discovery: BFS from the entry, resolving every import specifier.
    // ------------------------------------------------------------------
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(entry.clone());

    while let Some(path) = queue.pop_front() {
        if modules.contains_key(&path) {
            continue;
        }

        let source = match loader.load(&path) {
            Ok(src) => src,
            Err(msg) => {
                errors.push(diag(0, 0, format!("{}: {}", path.display(), msg)));
                continue;
            }
        };

        let meta = parse_source_module_meta(&source);
        let mut dependencies = HashMap::with_capacity(meta.imports.len());
        let mut virtual_imports = Vec::new();
        for import in &meta.imports {
            // Recorded before resolution so the set is complete even for
            // specifiers this loader cannot resolve.
            all_imports.insert(import.path.trim().to_string());
            if let Some(ui) = shake::normalize_ui_specifier(&import.path) {
                virtual_imports.push(ui);
                continue;
            }
            if is_compiler_ui_spec(&import.path) {
                continue;
            }
            match loader.resolve(&import.path, &path) {
                Ok(dep) => {
                    let from_ds = path
                        .extension()
                        .and_then(|e| e.to_str())
                        == Some("ds");
                    let to_dsx = dep
                        .extension()
                        .and_then(|e| e.to_str())
                        == Some("dsx");
                    if from_ds && to_dsx {
                        errors.push(diag(
                            0,
                            0,
                            format!(
                                "{}: `.ds` files cannot import `.dsx` modules (`{}`)",
                                path.display(),
                                import.path
                            ),
                        ));
                    }
                    dependencies.insert(import.path.clone(), dep.clone());
                    if !modules.contains_key(&dep) {
                        queue.push_back(dep);
                    }
                }
                Err(msg) => {
                    errors.push(diag(
                        0,
                        0,
                        format!("{}: cannot resolve '{}': {}", path.display(), import.path, msg),
                    ));
                }
            }
        }

        modules.insert(
            path.clone(),
            GraphModule {
                path,
                source,
                dependencies,
                virtual_imports,
            },
        );
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    // ------------------------------------------------------------------
    // Topological order (Kahn).  Cycles produce a diagnostic.
    // ------------------------------------------------------------------
    let order = topological_order(&modules).map_err(|cycle| {
        vec![diag(
            0,
            0,
            format!(
                "module import cycle detected: {}",
                cycle
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
        )]
    })?;

    // ------------------------------------------------------------------
    // Collect exported type information for every module first.  We keep the
    // parsed programs alive alongside the arena so importers can reference
    // dependency AST nodes safely.
    // ------------------------------------------------------------------
    let mut programs: HashMap<PathBuf, deka_syntax::Program> = HashMap::new();
    let mut exports: HashMap<PathBuf, deka_syntax::ModuleExports> =
        HashMap::with_capacity(modules.len());
    for module in modules.values() {
        let parse_result = deka_syntax::parse(&module.source, &arena);
        if let Some(program) = parse_result.program {
            programs.insert(module.path.clone(), program);
        }
    }
    for (path, program) in programs.iter() {
        exports.insert(path.clone(), deka_syntax::collect_module_exports(program, &arena));
    }

    // Build per-module import maps pointing to dependency exports.
    let mut imports: HashMap<PathBuf, HashMap<&str, &deka_syntax::ModuleExports>> =
        HashMap::with_capacity(modules.len());
    for module in modules.values() {
        let mut module_imports = HashMap::new();
        for (spec, dep) in module.dependencies.iter() {
            if let Some(dep_exports) = exports.get(dep) {
                module_imports.insert(spec.as_str(), dep_exports);
            }
        }
        imports.insert(module.path.clone(), module_imports);
    }

    // ------------------------------------------------------------------
    // Graph shaking: drop unused exports of pure modules, then unused modules.
    // Client entries that can reach ui/server fail the build.
    // ------------------------------------------------------------------
    let shake_modules: HashMap<PathBuf, ShakeModule> = modules
        .iter()
        .map(|(path, module)| {
            (
                path.clone(),
                ShakeModule {
                    dependencies: module.dependencies.clone(),
                    virtual_imports: module.virtual_imports.clone(),
                },
            )
        })
        .collect();
    let plan: ShakePlan = shake::shake_graph(&entry, &shake_modules, &programs);
    if options.client && plan.reaches_ui_server {
        errors.push(diag(
            0,
            0,
            format!(
                "{}: client bundle cannot import ui/server",
                entry.display()
            ),
        ));
        return Err(errors);
    }

    // ------------------------------------------------------------------
    // Emit each kept module.  We compile in dependency order so imported
    // structs, enums, and receiver methods are known to the typechecker.
    // ------------------------------------------------------------------
    let mut emitted: HashMap<PathBuf, String> = HashMap::with_capacity(plan.keep.len());
    for path in order {
        if !plan.keep.contains(&path) {
            continue;
        }
        let module = modules.get(&path).expect("module in graph");
        let input = path.to_string_lossy();
        let inferred = crate::infer_stdlib_imports_for_source(&module.source, &arena);
        let mut combined: HashMap<&str, &deka_syntax::ModuleExports> = HashMap::new();
        for (spec, exports) in &inferred {
            combined.insert(*spec, exports);
        }
        if let Some(graph_imports) = imports.get(&path) {
            for (spec, exports) in graph_imports {
                combined.insert(*spec, *exports);
            }
        }
        let compile_options = CompileOptions {
            used_exports: plan.live.get(&path).cloned().flatten(),
            client: options.client,
            ..Default::default()
        };
        match compile_to_js_with_imports_and_options(
            &module.source,
            &input,
            &arena,
            &combined,
            compile_options,
        ) {
            Ok(result) => {
                emitted.insert(path, result.js);
            }
            Err(diagnostics) => {
                for d in diagnostics {
                    errors.push(diag(
                        d.line,
                        d.column,
                        format!("{}: {}", path.display(), d.message),
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(ModuleGraphResult {
        entry,
        modules: emitted,
        imports: all_imports,
    })
}

fn diag(line: usize, column: usize, message: String) -> Diagnostic {
    deka_syntax::Diagnostic::error(line, column, message)
}

/// Compute a topological ordering of the discovered modules.  Returns the
/// first cycle found if the graph is not a DAG.
fn topological_order(modules: &HashMap<PathBuf, GraphModule>) -> Result<Vec<PathBuf>, Vec<PathBuf>> {
    let mut in_degree: HashMap<PathBuf, usize> = modules
        .keys()
        .map(|p| (p.clone(), 0))
        .collect();
    let mut adj: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

    for (path, module) in modules.iter() {
        for dep in module.dependencies.values() {
            if let Some(dep_module) = modules.get(dep) {
                // Only count edges to modules that were successfully loaded.
                *in_degree.entry(dep_module.path.clone()).or_insert(0) += 1;
                adj.entry(path.clone()).or_default().push(dep.clone());
            }
        }
    }

    let mut queue: VecDeque<PathBuf> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(p, _)| p.clone())
        .collect();
    let mut order: Vec<PathBuf> = Vec::with_capacity(modules.len());

    while let Some(path) = queue.pop_front() {
        order.push(path.clone());
        for dep in adj.get(&path).into_iter().flatten() {
            let deg = in_degree.get_mut(dep).expect("in-degree entry");
            *deg -= 1;
            if *deg == 0 {
                queue.push_back(dep.clone());
            }
        }
    }

    if order.len() != modules.len() {
        // Return one cycle using DFS.
        return Err(find_cycle(modules));
    }

    Ok(order)
}

fn find_cycle(modules: &HashMap<PathBuf, GraphModule>) -> Vec<PathBuf> {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<PathBuf> = Vec::new();
    let mut on_stack: HashSet<PathBuf> = HashSet::new();

    for start in modules.keys() {
        if visited.contains(start) {
            continue;
        }
        if let Some(cycle) = dfs_cycle(start, modules, &mut visited, &mut stack, &mut on_stack) {
            return cycle;
        }
    }

    Vec::new()
}

fn dfs_cycle(
    node: &PathBuf,
    modules: &HashMap<PathBuf, GraphModule>,
    visited: &mut HashSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
    on_stack: &mut HashSet<PathBuf>,
) -> Option<Vec<PathBuf>> {
    visited.insert(node.clone());
    stack.push(node.clone());
    on_stack.insert(node.clone());

    for dep in modules.get(node)?.dependencies.values() {
        if !modules.contains_key(dep) {
            continue;
        }
        if !visited.contains(dep) {
            if let Some(cycle) = dfs_cycle(dep, modules, visited, stack, on_stack) {
                return Some(cycle);
            }
        } else if on_stack.contains(dep) {
            // Found cycle; slice from dep to end of stack.
            let idx = stack.iter().position(|p| p == dep).unwrap_or(0);
            let mut cycle = stack[idx..].to_vec();
            cycle.push(dep.clone());
            return Some(cycle);
        }
    }

    stack.pop();
    on_stack.remove(node);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct InMemoryLoader {
        files: HashMap<PathBuf, String>,
        aliases: HashMap<(PathBuf, String), PathBuf>,
    }

    impl ModuleLoader for InMemoryLoader {
        fn resolve(&self, specifier: &str, referrer: &Path) -> Result<PathBuf, String> {
            let key = (referrer.to_path_buf(), specifier.to_string());
            self.aliases
                .get(&key)
                .cloned()
                .ok_or_else(|| format!("unmapped specifier {} from {}", specifier, referrer.display()))
        }

        fn load(&self, path: &Path) -> Result<String, String> {
            self.files
                .get(path)
                .cloned()
            .ok_or_else(|| format!("missing in-memory file {}", path.display()))
        }
    }

    #[test]
    fn graph_compiles_relative_imports() {
        let root = PathBuf::from("/project");
        let math = root.join("math.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            math.clone(),
            "export fn add(a: number, b: number): number { return a + b; }".to_string(),
        );
        files.insert(
            main.clone(),
            "import { add } from \"./math.ds\";\nconst r: number = add(1, 2);".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./math.ds".to_string()), math.clone());

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        assert_eq!(result.modules.len(), 2);
        assert!(result.modules[&main].contains("add(1, 2)"));
        assert!(result.modules[&math].contains("function add"));
    }

    #[test]
    fn graph_rejects_cycles() {
        let root = PathBuf::from("/project");
        let a = root.join("a.ds");
        let b = root.join("b.ds");

        let mut files = HashMap::new();
        files.insert(a.clone(), "import { x } from \"./b.ds\";".to_string());
        files.insert(b.clone(), "import { x } from \"./a.ds\";".to_string());

        let mut aliases = HashMap::new();
        aliases.insert((a.clone(), "./b.ds".to_string()), b.clone());
        aliases.insert((b.clone(), "./a.ds".to_string()), a.clone());

        let loader = InMemoryLoader { files, aliases };
        let err = compile_module_graph(&a, &loader).expect_err("cycle should fail");
        assert!(err.iter().any(|d| d.message.contains("cycle")));
    }

    #[test]
    fn graph_compiles_cross_module_struct() {
        let root = PathBuf::from("/project");
        let person = root.join("person.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            person.clone(),
            "struct Person { name: string }\nfn (p Person) greet(): string { return p.name }\nexport { Person }".to_string(),
        );
        files.insert(
            main.clone(),
            "import { Person } from \"./person.ds\";\nconst p = Person { name: \"Deka\" };\nconst g: string = p.greet();".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./person.ds".to_string()), person.clone());

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        assert_eq!(result.modules.len(), 2);
        let person_js = &result.modules[&person];
        let main_js = &result.modules[&main];
        assert!(person_js.contains("const Person = deka.Struct(\"Person\")"), "got: {}", person_js);
        assert!(person_js.contains("Person.impl(\"greet\""), "got: {}", person_js);
        assert!(person_js.contains("export { Person };"), "got: {}", person_js);
        assert!(main_js.contains("import { Person } from \"./person.ds\";"), "got: {}", main_js);
        assert!(main_js.contains("Person({ name: \"Deka\" })"), "got: {}", main_js);
        assert!(main_js.contains("p.greet()"), "got: {}", main_js);
    }

    #[test]
    fn graph_compiles_cross_module_enum() {
        let root = PathBuf::from("/project");
        let color = root.join("color.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            color.clone(),
            "enum Color { Red, Green, Blue }\nexport { Color }".to_string(),
        );
        files.insert(
            main.clone(),
            "import { Color } from \"./color.ds\";\nconst c: Color = Color.Red;\nconst label: string = match c { Red => \"red\", _ => \"other\" };".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./color.ds".to_string()), color.clone());

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        assert_eq!(result.modules.len(), 2);
        let color_js = &result.modules[&color];
        let main_js = &result.modules[&main];
        assert!(color_js.contains("const Color = Object.freeze"), "got: {}", color_js);
        assert!(color_js.contains("export { Color };"), "got: {}", color_js);
        assert!(main_js.contains("import { Color } from \"./color.ds\";"), "got: {}", main_js);
        assert!(main_js.contains("Color.Red"), "got: {}", main_js);
        assert!(main_js.contains("__deka_scrutinee"), "got: {}", main_js);
    }

    #[test]
    fn graph_compiles_cross_module_newtype_method() {
        let root = PathBuf::from("/project");
        let money = root.join("money.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            money.clone(),
            "type Cents number\nfn (c Cents) toDollars() number { return number(c) / 100 }\nexport { Cents }".to_string(),
        );
        files.insert(
            main.clone(),
            "import { Cents } from \"./money.ds\";\nconst c: Cents = Cents(500);\nconst d: number = c.toDollars();".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./money.ds".to_string()), money.clone());

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        assert_eq!(result.modules.len(), 2);
        let money_js = &result.modules[&money];
        let main_js = &result.modules[&main];
        assert!(money_js.contains("const Cents$proto"), "got: {}", money_js);
        assert!(
            money_js.contains("Cents$proto.toDollars = function()"),
            "got: {}",
            money_js
        );
        assert!(money_js.contains("export { Cents };"), "got: {}", money_js);
        assert!(main_js.contains("import { Cents } from \"./money.ds\";"), "got: {}", main_js);
        assert!(main_js.contains("Cents(500)"), "got: {}", main_js);
        assert!(main_js.contains("c.toDollars()"), "got: {}", main_js);
    }

    #[test]
    fn graph_drops_unused_export_from_pure_module() {
        let root = PathBuf::from("/project");
        let lib = root.join("lib.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            lib.clone(),
            "export fn keep() { return \"KEEP_ME\"; }\nexport fn drop() { return \"DROP_ME_UNIQUE\"; }".to_string(),
        );
        files.insert(
            main.clone(),
            "import { keep } from \"./lib.ds\";\nconst x: string = keep();".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./lib.ds".to_string()), lib.clone());

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        assert_eq!(result.modules.len(), 2);
        let lib_js = &result.modules[&lib];
        assert!(lib_js.contains("KEEP_ME"), "got: {}", lib_js);
        assert!(
            !lib_js.contains("DROP_ME_UNIQUE"),
            "unused export should be shaken: {}",
            lib_js
        );
    }

    #[test]
    fn graph_keeps_impure_module_side_effect() {
        let root = PathBuf::from("/project");
        let logger = root.join("logger.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            logger.clone(),
            "const marker: string = \"SIDE_EFFECT_UNIQUE\";".to_string(),
        );
        files.insert(
            main.clone(),
            "import \"./logger.ds\";\nconst x: number = 1;".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./logger.ds".to_string()), logger.clone());

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        assert_eq!(result.modules.len(), 2);
        let logger_js = &result.modules[&logger];
        assert!(
            logger_js.contains("SIDE_EFFECT_UNIQUE"),
            "impure module must be kept: {}",
            logger_js
        );
    }

    #[test]
    fn client_graph_rejects_ui_server() {
        let root = PathBuf::from("/project");
        let main = root.join("main.dsx");

        let mut files = HashMap::new();
        files.insert(
            main.clone(),
            "import { renderToString } from \"ui/server\";\nexport const x = renderToString;".to_string(),
        );

        let loader = InMemoryLoader {
            files,
            aliases: HashMap::new(),
        };
        let err = compile_module_graph_with_options(
            &main,
            &loader,
            GraphCompileOptions { client: true },
        )
        .expect_err("ui/server on a client entry");
        assert!(
            err.iter().any(|d| d.message.contains("ui/server")),
            "{:?}",
            err
        );
    }

    #[test]
    fn graph_compiles_cross_module_struct_embed() {
        let root = PathBuf::from("/project");
        let legs = root.join("legs.ds");
        let robot = root.join("robot.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            legs.clone(),
            "struct Legs {}\nfn (l Legs) move() string { return \"walk\" }\nexport { Legs }".to_string(),
        );
        files.insert(
            robot.clone(),
            "import { Legs } from \"./legs.ds\";\nstruct Robot { Legs }\nexport { Robot }".to_string(),
        );
        files.insert(
            main.clone(),
            "import { Robot } from \"./robot.ds\";\nimport { Legs } from \"./legs.ds\";\nconst r = Robot { Legs: Legs {} };\nconst m: string = r.move();".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((robot.clone(), "./legs.ds".to_string()), legs.clone());
        aliases.insert((main.clone(), "./robot.ds".to_string()), robot.clone());
        aliases.insert((main.clone(), "./legs.ds".to_string()), legs.clone());

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        assert_eq!(result.modules.len(), 3);
        let robot_js = &result.modules[&robot];
        let main_js = &result.modules[&main];
        assert!(robot_js.contains("Robot = deka.Struct(\"Robot\"") && robot_js.contains("{ Legs: Legs }"), "got: {}", robot_js);
        assert!(main_js.contains("Robot({ Legs: Legs({"), "got: {}", main_js);
        assert!(main_js.contains("r.move()"), "got: {}", main_js);
    }

    #[test]
    fn graph_propagates_result_type_for_imported_function() {
        let root = PathBuf::from("/project");
        let crypto = root.join("crypto.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            crypto.clone(),
            "export fn random_bytes(n: number): Result<string, string> {\n  return unsafe { String(n) }\n}".to_string(),
        );
        files.insert(
            main.clone(),
            "import { random_bytes } from \"./crypto.ds\";\nconst r = match (random_bytes(32)) { Ok(v) => v, Err(e) => \"\" };".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./crypto.ds".to_string()), crypto.clone());

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        assert_eq!(result.modules.len(), 2);
        let main_js = &result.modules[&main];
        assert!(main_js.contains("random_bytes(32)"), "got: {}", main_js);
        assert!(main_js.contains("__case"), "got: {}", main_js);
    }

    #[test]
    fn graph_propagates_declared_return_type_for_imported_generic_function() {
        let root = PathBuf::from("/project");
        let lib = root.join("lib.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            lib.clone(),
            "export fn generic<T>(payload: T) Result<string, string> {\n  return Ok(\"y\")\n}".to_string(),
        );
        files.insert(
            main.clone(),
            "import { generic } from \"./lib.ds\";\nconst value: string = match (generic({sub: \"1\"})) { Ok(v) => v, Err(e) => e };".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./lib.ds".to_string()), lib);

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        let main_js = &result.modules[&main];
        assert!(main_js.contains("generic({sub: \"1\"})"), "got: {}", main_js);
        assert!(main_js.contains("__case"), "got: {}", main_js);
    }

    #[test]
    fn graph_substitutes_imported_generic_return_type() {
        let root = PathBuf::from("/project");
        let lib = root.join("lib.ds");
        let main = root.join("main.ds");

        let mut files = HashMap::new();
        files.insert(
            lib.clone(),
            "export fn first<T>(values: Array<T>) Option<T> {\n  return Some(values[0])\n}".to_string(),
        );
        files.insert(
            main.clone(),
            "import { first } from \"./lib.ds\";\nconst value: string = match (first([\"x\"])) { Some(v) => v, None => \"\" };".to_string(),
        );

        let mut aliases = HashMap::new();
        aliases.insert((main.clone(), "./lib.ds".to_string()), lib);

        let loader = InMemoryLoader { files, aliases };
        let result = compile_module_graph(&main, &loader).expect("compile graph");
        let main_js = &result.modules[&main];
        assert!(main_js.contains("first([\"x\"])"), "got: {}", main_js);
        assert!(main_js.contains("__case"), "got: {}", main_js);
    }

    #[test]
    fn fs_loader_resolves_relative_and_index() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(src.join("lib")).unwrap();
        std::fs::write(src.join("math.ds"), "export fn add() {}").unwrap();
        std::fs::write(src.join("lib").join("index.ds"), "export const x = 1;").unwrap();

        let loader = FsModuleLoader::new(root.clone());
        let referrer = src.join("main.ds");
        assert_eq!(
            loader.resolve("./math.ds", &referrer).unwrap(),
            std::fs::canonicalize(src.join("math.ds")).unwrap()
        );
        assert_eq!(
            loader.resolve("./lib", &referrer).unwrap(),
            std::fs::canonicalize(src.join("lib").join("index.ds")).unwrap()
        );
    }

    #[test]
    fn fs_loader_resolves_bare_stdlib_in_ds_modules() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let ds_modules = root.join("ds_modules");
        std::fs::create_dir_all(ds_modules.join("json")).unwrap();
        std::fs::write(ds_modules.join("json").join("index.ds"), "export fn parse() {}").unwrap();

        let loader = FsModuleLoader::new(root.clone());
        let referrer = root.join("main.ds");
        let resolved = loader.resolve("json", &referrer).unwrap();
        assert_eq!(
            std::fs::canonicalize(&resolved).unwrap(),
            std::fs::canonicalize(ds_modules.join("json").join("index.ds")).unwrap()
        );
    }

    #[test]
    fn fs_loader_prefers_local_link_over_installed_package() {
        let project = tempfile::tempdir().expect("project");
        let package = tempfile::tempdir().expect("package");
        let installed = project.path().join("ds_modules/@deka/example");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("index.ds"), "export fn source() {}\n").unwrap();
        std::fs::write(package.path().join("index.ds"), "export fn source() {}\n").unwrap();
        runtime_core::modules::write_links_at(
            project.path(),
            &runtime_core::modules::LinkManifest {
                version: runtime_core::modules::LINKS_VERSION,
                packages: std::collections::BTreeMap::from([(
                    "@deka/example".to_string(),
                    runtime_core::modules::LinkEntry {
                        path: package.path().canonicalize().unwrap(),
                    },
                )]),
            },
        )
        .unwrap();

        let loader = FsModuleLoader::new(project.path().to_path_buf());
        let resolved = loader
            .resolve("example", &project.path().join("main.ds"))
            .unwrap();
        assert!(resolved.starts_with(package.path().canonicalize().unwrap()));
    }

    #[test]
    fn graph_treats_ui_runtime_as_external() {
        let main = PathBuf::from("/project/page.dsx");
        let mut files = HashMap::new();
        files.insert(
            main.clone(),
            "import { Form } from \"ui/form\";\nconst el = <Form action=\"/api/x\" method=\"post\">Go</Form>;\n"
                .to_string(),
        );
        let loader = InMemoryLoader {
            files,
            aliases: HashMap::new(),
        };
        let result =
            compile_module_graph(&main, &loader).expect("ui/form should not need a .ds module");
        let js = &result.modules[&main];
        assert!(js.contains("ui/form"), "got: {js}");
        assert!(js.contains("Form"), "got: {js}");
    }

    #[test]
    fn generated_defer_entry_compiles() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_defer_compile_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("app/blog")).unwrap();
        std::fs::write(tmp.join("deka.json"), "{}\n").unwrap();
        std::fs::write(
            tmp.join("app/blog/page.dsx"),
            "export fn Post() {\n    return <article>blog-secret</article>;\n}\nexport fn Page() {\n    return <main><Post server:defer><span slot=\"fallback\">loading-post</span></Post></main>;\n}\n",
        )
        .unwrap();
        let entry = runtime_core::framework::write_defer_router_entry(&tmp)
            .expect("write defer-entry");
        let loader = FsModuleLoader::new(tmp.clone());
        if let Err(errs) = compile_module_graph(&entry, &loader) {
            let _ = std::fs::remove_dir_all(&tmp);
            panic!(
                "defer-entry failed to compile:\n{}",
                errs.iter()
                    .map(|d| format!("{}:{}: {}", d.line, d.column, d.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn compile_or_panic(entry: &Path, root: &Path, label: &str) {
        let loader = FsModuleLoader::new(root.to_path_buf());
        if let Err(errs) = compile_module_graph(entry, &loader) {
            panic!(
                "{label} failed to compile:\n{}",
                errs.iter()
                    .map(|d| format!("{}:{}: {}", d.line, d.column, d.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }

    #[test]
    fn generated_middleware_and_api_entries_compile() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_mw_api_compile_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("api/boom")).unwrap();
        std::fs::write(tmp.join("deka.json"), "{}\n").unwrap();
        std::fs::write(
            tmp.join("middleware.ds"),
            r#"export const matcher = ["/_deka/defer"]
interface RequestHeaders { accept: string }
interface ResponseHeaders { location: string }
interface Request { url: string, pathname: string, method: string, headers: RequestHeaders }
interface Response { status: number, body: string, headers: ResponseHeaders }
export fn middleware(request: Request): Option<Response> {
    return None
}
"#,
        )
        .unwrap();
        std::fs::write(
            tmp.join("api/boom/route.ds"),
            r#"interface RequestHeaders { accept: string }
interface Request { url: string, pathname: string, method: string, headers: RequestHeaders }
interface Response { status: number, body: string }
export fn GET(request: Request): Response {
    return { status: 200, body: "ok" }
}
"#,
        )
        .unwrap();
        let mw = runtime_core::framework::write_middleware_router_entry(&tmp)
            .expect("write middleware-entry");
        compile_or_panic(&mw, &tmp, "middleware-entry");
        let api =
            runtime_core::framework::write_api_router_entry(&tmp).expect("write api-entry");
        compile_or_panic(&api, &tmp, "api-entry");
        let worker =
            runtime_core::framework::write_worker_router_entry(&tmp).expect("write worker-entry");
        compile_or_panic(&worker, &tmp, "worker-entry");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
