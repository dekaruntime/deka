//! Cross-file module graph compilation for DekaScript compiler v2.
//!
//! This module discovers all reachable `.ds` files from an entry point,
//! resolves relative and bare stdlib specifiers, and compiles each module
//! through the v2 pipeline.  Exported structs, enums, type aliases, and
//! receiver methods are propagated through the module graph so importers can
//! construct imported structs and match imported enums with full typechecking.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use bumpalo::Bump;
use deka_syntax::Diagnostic;

use crate::{compile_to_js_with_imports, parse_source_module_meta};

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
/// 4. Bare specifier (e.g. `json`, `@deka/crypto`) → `ds_modules/` (with
///    `@deka/` aliases), falling back to `DEKA_MODULE_ROOT` if set.
pub struct FsModuleLoader {
    project_root: PathBuf,
}

impl FsModuleLoader {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    fn resolve_ds_file(&self, base: &Path) -> Option<PathBuf> {
        runtime_core::module_spec::resolve_ds_source_file(base)
    }

    fn guard_project_root(&self, path: &Path) -> Result<PathBuf, String> {
        let canon_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let canon_root = std::fs::canonicalize(&self.project_root)
            .unwrap_or_else(|_| self.project_root.clone());
        if !canon_path.starts_with(&canon_root) {
            return Err(format!(
                "import escapes project root: {} is outside {}",
                canon_path.display(),
                canon_root.display()
            ));
        }
        Ok(canon_path)
    }
}

impl ModuleLoader for FsModuleLoader {
    fn resolve(&self, specifier: &str, referrer: &Path) -> Result<PathBuf, String> {
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

        // DEKA_MODULE_ROOT fallback for stdlib-only tenants.
        if let Some(root_os) = std::env::var_os("DEKA_MODULE_ROOT") {
            let root = PathBuf::from(root_os);
            for alias in &aliases {
                let base = root.join(alias);
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

/// A discovered module and its outgoing dependencies.
#[derive(Debug)]
struct GraphModule {
    path: PathBuf,
    source: String,
    /// Resolved dependency path for each import specifier in this module.
    dependencies: HashMap<String, PathBuf>,
}

/// Result of compiling a module graph.
#[derive(Debug)]
pub struct ModuleGraphResult {
    /// Absolute path of the entry module.
    pub entry: PathBuf,
    /// Map from absolute module path to emitted JavaScript.
    pub modules: HashMap<PathBuf, String>,
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
    let entry = std::fs::canonicalize(entry).unwrap_or_else(|_| entry.to_path_buf());
    let arena = Bump::new();

    let mut modules: HashMap<PathBuf, GraphModule> = HashMap::new();
    let mut errors: Vec<Diagnostic> = Vec::new();

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
        for import in &meta.imports {
            match loader.resolve(&import.path, &path) {
                Ok(dep) => {
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
    // Emit each module.  We compile in dependency order so imported structs,
    // enums, and receiver methods are known to the typechecker.
    // ------------------------------------------------------------------
    let mut emitted: HashMap<PathBuf, String> = HashMap::with_capacity(modules.len());
    for path in order {
        let module = modules.get(&path).expect("module in graph");
        let input = path.to_string_lossy();
        let module_imports = imports.get(&path).cloned().unwrap_or_default();
        match compile_to_js_with_imports(&module.source, &input, &arena, &module_imports) {
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

    Ok(ModuleGraphResult { entry, modules: emitted })
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
            "struct Person { name: string }\nfn (p Person) greet(): string { return this.name }\nexport { Person }".to_string(),
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
}
