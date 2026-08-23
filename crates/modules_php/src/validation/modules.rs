use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::integrity::compute_package_integrity;
use bumpalo::Bump;
use php_rs::parser::ast::visitor::{Visitor, walk_expr};
use php_rs::parser::ast::{Expr, ExprId, Program, Stmt};
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use serde_json::Value;

use runtime_core::modules::{existing_modules_dirs, is_modules_dir_name, MODULES_DIR};

use super::{ErrorKind, Severity, ValidationError};
use crate::validation::exports::{parse_export_function, parse_export_list_line};
use crate::validation::imports::{
    ImportKind, ImportSpec, consume_comment_line, frontmatter_bounds, is_ident, parse_import_line,
    strip_php_tags_inline,
};

#[derive(Debug, Clone)]
struct ModuleNode {
    module_id: String,
    imports: Vec<ImportEdge>,
    exports: HashSet<String>,
    has_top_level_await: bool,
}

#[derive(Debug, Clone)]
struct ImportEdge {
    module_id: String,
    imported: String,
    line: usize,
    column: usize,
    raw_from: String,
}

pub fn validate_module_resolution(source: &str, file_path: &str) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let modules_root = resolve_modules_root(file_path);
    let imports = collect_import_specs(source, file_path);
    let available_modules = modules_root
        .as_deref()
        .map(scan_phpx_modules)
        .unwrap_or_default();

    let mut graph = ModuleGraph::new(
        modules_root.clone(),
        available_modules.clone(),
    );
    if !imports.is_empty() {
        graph.ensure_loaded("<entry>", Path::new(file_path), &mut errors);
    }

    if !errors.is_empty() {
        return errors;
    }

    graph.collect_missing_exports(&mut errors);
    graph.detect_cycles(&mut errors);
    if let Some(root) = modules_root.as_deref() {
        let package_errors = validate_package_integrity(root, &graph.package_integrity_targets);
        errors.extend(package_errors);
    }
    errors
}

pub fn validate_wasm_imports(source: &str, file_path: &str) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let modules_root = resolve_modules_root(file_path);
    let imports = collect_import_specs(source, file_path);
    let available_wasm = modules_root
        .as_deref()
        .map(scan_wasm_modules)
        .unwrap_or_default();

    for spec in imports {
        if spec.kind != ImportKind::Wasm {
            continue;
        }
        if spec.from.starts_with('@')
            && !spec.from.starts_with("@/")
            && !is_valid_user_module(&spec.from)
        {
            errors.push(wasm_error(
                spec.line,
                spec.column,
                spec.from.len().max(1),
                format!("Invalid wasm module id '{}'.", spec.from),
                "Use '@user/module' format for user wasm modules.",
            ));
            continue;
        }
        match resolve_wasm_target(
            &spec.from,
            file_path,
            modules_root.as_deref(),
            Some(&available_wasm),
        ) {
            Ok(target) => {
                validate_wasm_manifest(&target, &spec, &mut errors);
            }
            Err(err) => errors.push(err),
        }
    }

    errors
}

pub fn validate_target_capabilities(source: &str, file_path: &str) -> Vec<ValidationError> {
    let target = std::env::var("DEKA_TARGET")
        .ok()
        .or_else(|| std::env::var("DEKA_HOST_PROFILE").ok())
        .unwrap_or_else(|| "server".to_string())
        .to_ascii_lowercase();
    if target != "adwa" {
        return Vec::new();
    }

    let mut errors = Vec::new();
    for spec in collect_import_specs(source, file_path) {
        if spec.kind == ImportKind::Wasm {
            continue;
        }
        if let Some((capability, reason, suggestion)) = adwa_capability_block(&spec.from) {
            errors.push(module_error(
                spec.line,
                spec.column,
                spec.from.len().max(1),
                format!(
                    "Target capability error: module '{}' is unavailable for target 'adwa' ({}).",
                    spec.from, reason
                ),
                &format!(
                    "Switch target or avoid {} APIs in browser-targeted modules. {}",
                    capability, suggestion
                ),
            ));
        }
    }
    errors
}

struct ModuleGraph {
    modules_root: Option<PathBuf>,
    available_modules: HashSet<String>,
    package_integrity_targets: HashMap<String, PathBuf>,
    nodes: HashMap<String, ModuleNode>,
}

impl ModuleGraph {
    fn new(
        modules_root: Option<PathBuf>,
        available_modules: HashSet<String>,
    ) -> Self {
        Self {
            modules_root,
            available_modules,
            package_integrity_targets: HashMap::new(),
            nodes: HashMap::new(),
        }
    }

    fn ensure_loaded(
        &mut self,
        module_id: &str,
        file_path: &Path,
        errors: &mut Vec<ValidationError>,
    ) {
        if self.nodes.contains_key(module_id) {
            return;
        }

        // Insert a placeholder before traversing imports so recursive/cyclic
        // graphs don't recurse forever while loading.
        self.nodes.insert(
            module_id.to_string(),
            ModuleNode {
                module_id: module_id.to_string(),
                imports: Vec::new(),
                exports: HashSet::new(),
                has_top_level_await: false,
            },
        );

        let source = match std::fs::read_to_string(file_path) {
            Ok(src) => src,
            Err(err) => {
                errors.push(module_error(
                    1,
                    1,
                    1,
                    format!("Failed to read module '{}': {}", module_id, err),
                    "Ensure the module file exists and is readable.",
                ));
                self.nodes.remove(module_id);
                return;
            }
        };

        let mut imports = Vec::new();
        let import_specs = collect_import_specs(&source, file_path.to_string_lossy().as_ref());
        for spec in import_specs {
            if spec.kind == ImportKind::Wasm {
                continue;
            }
            if spec.from.starts_with('@')
                && !spec.from.starts_with("@/")
                && !is_valid_user_module(&spec.from)
            {
                errors.push(module_error(
                    spec.line,
                    spec.column,
                    spec.from.len().max(1),
                    format!("Invalid module id '{}'.", spec.from),
                    "Use '@user/module' format for user modules.",
                ));
                continue;
            }
            match resolve_import_target(
                &spec.from,
                file_path.to_string_lossy().as_ref(),
                self.modules_root.as_deref(),
                Some(&self.available_modules),
            ) {
                Ok(resolved) => {
                    if let Some(target) = resolved.integrity_target {
                        self.package_integrity_targets
                            .insert(target.name, target.package_root);
                    }
                    imports.push(ImportEdge {
                        module_id: resolved.module_id.clone(),
                        imported: spec.imported.clone(),
                        line: spec.line,
                        column: spec.column,
                        raw_from: spec.from.clone(),
                    });
                    self.ensure_loaded(&resolved.module_id, &resolved.file_path, errors);
                }
                Err(err) => errors.push(err),
            }
        }

        let mut exports = collect_exports(&source, file_path.to_string_lossy().as_ref());
        if is_template_module(&source) {
            exports.insert("Component".to_string());
        }

        if let Some(node) = self.nodes.get_mut(module_id) {
            node.imports = imports;
            node.exports = exports;
            node.has_top_level_await = has_top_level_await(&source);
        }
    }

    fn collect_missing_exports(&self, errors: &mut Vec<ValidationError>) {
        for node in self.nodes.values() {
            for edge in &node.imports {
                let Some(target) = self.nodes.get(&edge.module_id) else {
                    errors.push(module_error(
                        edge.line,
                        edge.column,
                        edge.raw_from.len().max(1),
                        format!(
                            "Unknown phpx module '{}' imported by '{}'.",
                            edge.raw_from, node.module_id
                        ),
                        "Ensure the module exists in ds_modules/.",
                    ));
                    continue;
                };
                if !target.exports.contains(&edge.imported) {
                    errors.push(module_error(
                        edge.line,
                        edge.column,
                        edge.imported.len().max(1),
                        format!(
                            "Missing export '{}' in '{}' (imported by '{}').",
                            edge.imported, edge.raw_from, node.module_id
                        ),
                        "Export the symbol from the module or update the import.",
                    ));
                }
            }
        }
    }

    fn detect_cycles(&self, errors: &mut Vec<ValidationError>) {
        let mut stack = Vec::new();
        let mut visited = HashSet::new();
        let mut reported = HashSet::new();

        for module_id in self.nodes.keys() {
            self.visit(module_id, &mut stack, &mut visited, &mut reported, errors);
        }
    }

    fn visit(
        &self,
        module_id: &str,
        stack: &mut Vec<String>,
        visited: &mut HashSet<String>,
        reported: &mut HashSet<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        if visited.contains(module_id) {
            return;
        }
        if let Some(pos) = stack.iter().position(|entry| entry == module_id) {
            let mut cycle = stack[pos..].to_vec();
            cycle.push(module_id.to_string());
            let cycle_key = cycle.join(" -> ");
            if !reported.insert(cycle_key.clone()) {
                return;
            }
            let has_tla = cycle.iter().any(|id| {
                self.nodes
                    .get(id.as_str())
                    .map(|node| node.has_top_level_await)
                    .unwrap_or(false)
            });
            let (message, help) = if has_tla {
                (
                    format!("Top-level await import cycle detected: {}", cycle_key),
                    "Break the async cycle by removing one import edge or by moving await out of module scope.",
                )
            } else {
                (
                    format!("Cyclic phpx import detected: {}", cycle_key),
                    "Break the cycle by removing one of the imports.",
                )
            };
            errors.push(module_error(1, 1, module_id.len().max(1), message, help));
            return;
        }
        stack.push(module_id.to_string());
        if let Some(node) = self.nodes.get(module_id) {
            for edge in &node.imports {
                self.visit(&edge.module_id, stack, visited, reported, errors);
            }
        }
        stack.pop();
        visited.insert(module_id.to_string());
    }
}

struct AwaitFinder {
    found: bool,
}

impl<'ast> Visitor<'ast> for AwaitFinder {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        if self.found {
            return;
        }
        if matches!(*expr, Expr::Await { .. }) {
            self.found = true;
            return;
        }
        walk_expr(self, expr);
    }
}

fn expr_has_await(expr: ExprId<'_>) -> bool {
    let mut finder = AwaitFinder { found: false };
    finder.visit_expr(expr);
    finder.found
}

fn stmt_is_tla_candidate(stmt: &Stmt<'_>) -> bool {
    match stmt {
        Stmt::Expression { expr, .. } => expr_has_await(*expr),
        Stmt::Return {
            expr: Some(expr), ..
        } => expr_has_await(*expr),
        Stmt::Echo { exprs, .. } => exprs.iter().any(|expr| expr_has_await(*expr)),
        _ => false,
    }
}

fn has_top_level_await(source: &str) -> bool {
    let filtered = source
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("import ") || trimmed.starts_with("export "))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let arena = Bump::new();
    let mut parser =
        Parser::new_with_mode(Lexer::new(filtered.as_bytes()), &arena, ParserMode::Ds);
    let program: Program<'_> = parser.parse_program();
    program
        .statements
        .iter()
        .any(|stmt| stmt_is_tla_candidate(stmt))
}

pub(crate) fn collect_import_specs(source: &str, file_path: &str) -> Vec<ImportSpec> {
    let lines: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_bounds(&lines);
    let scan_end = bounds.map(|(_, end)| end).unwrap_or(lines.len());
    let mut in_block_comment = false;
    let mut specs = Vec::new();
    for (idx, line) in lines.iter().enumerate().take(scan_end) {
        if let Some((start, end)) = bounds {
            if idx == start || idx == end {
                continue;
            }
        }
        let clean = strip_php_tags_inline(line);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            continue;
        }
        if consume_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }
        if trimmed.starts_with("import ") {
            if let Ok(mut parsed) = parse_import_line(trimmed, line, idx + 1, file_path) {
                specs.append(&mut parsed);
            }
        }
    }
    specs
}

fn collect_exports(source: &str, file_path: &str) -> HashSet<String> {
    let lines: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_bounds(&lines);
    let scan_end = bounds.map(|(_, end)| end).unwrap_or(lines.len());
    let mut in_block_comment = false;
    let mut exports = HashSet::new();
    for (idx, line) in lines.iter().enumerate().take(scan_end) {
        if let Some((start, end)) = bounds {
            if idx == start || idx == end {
                continue;
            }
        }
        let clean = strip_php_tags_inline(line);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            continue;
        }
        if consume_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }
        if trimmed.starts_with("export function")
            || trimmed.starts_with("export async function")
            || trimmed.starts_with("export fn")
            || trimmed.starts_with("export async fn")
        {
            if let Ok(spec) = parse_export_function(trimmed, line, idx + 1, file_path) {
                exports.insert(spec.name);
            }
            continue;
        }
        if trimmed.starts_with("export const ") {
            let rest = trimmed.trim_start_matches("export const ").trim_start();
            if let Some(name) = rest.split(|ch: char| ch == '=' || ch.is_whitespace()).next() {
                if is_ident(name) {
                    exports.insert(name.to_string());
                }
            }
            continue;
        }
        if trimmed.starts_with("export {") {
            if let Ok(specs) = parse_export_list_line(trimmed, line, idx + 1, file_path) {
                for spec in specs {
                    exports.insert(spec.name);
                }
            }
        }
    }
    exports
}

fn is_template_module(source: &str) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_bounds(&lines);
    let Some((_, end)) = bounds else {
        return false;
    };
    lines
        .iter()
        .skip(end + 1)
        .any(|line| !line.trim().is_empty())
}

pub(crate) fn resolve_modules_root(file_path: &str) -> Option<PathBuf> {
    let env_val = std::env::var("DEKA_MODULE_ROOT").ok();
    resolve_modules_root_with_env(file_path, env_val.as_deref())
}

fn resolve_modules_root_with_env(
    file_path: &str,
    env_module_root: Option<&str>,
) -> Option<PathBuf> {
    let path = Path::new(file_path);
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    if let Some(root) = find_project_root(&dir) {
        if let Some(candidate) = existing_modules_dirs(&root).into_iter().next() {
            return Some(candidate);
        }
    }

    if let Some(env_root) = env_module_root {
        let root = PathBuf::from(env_root);
        if root.join("deka.lock").exists() {
            if let Some(candidate) = existing_modules_dirs(&root).into_iter().next() {
                return Some(candidate);
            }
        }
        if root
            .file_name()
            .is_some_and(|name| is_modules_dir_name(&name.to_string_lossy()))
            && root.exists()
            && root
                .parent()
                .is_some_and(|parent| parent.join("deka.lock").exists())
        {
            return Some(root);
        }
        return None;
    }

    for ancestor in dir.ancestors() {
        if ancestor
            .file_name()
            .is_some_and(|name| is_modules_dir_name(&name.to_string_lossy()))
        {
            return Some(ancestor.to_path_buf());
        }
        if let Some(candidate) = existing_modules_dirs(ancestor).into_iter().next() {
            return Some(candidate);
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        if let Some(root) = find_project_root(&current_dir) {
            if let Some(candidate) = existing_modules_dirs(&root).into_iter().next() {
                return Some(candidate);
            }
        }
        for ancestor in current_dir.ancestors() {
            if let Some(candidate) = existing_modules_dirs(ancestor).into_iter().next() {
                return Some(candidate);
            }
        }
    }
    None
}

fn scan_phpx_modules(modules_root: &Path) -> HashSet<String> {
    let mut modules = HashSet::new();
    let mut stack = vec![modules_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == ".cache" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) == Some("phpx") {
                if let Ok(rel) = path.strip_prefix(modules_root) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    modules.insert(module_id_from_rel(&rel));
                }
            }
        }
    }
    modules
}

fn scan_wasm_modules(modules_root: &Path) -> HashSet<String> {
    let mut modules = HashSet::new();
    let mut stack = vec![modules_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == ".cache" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if name == "deka.json" {
                if let Some(parent) = path.parent() {
                    if let Ok(rel) = parent.strip_prefix(modules_root) {
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        if !rel.is_empty() {
                            modules.insert(rel);
                        }
                    }
                }
            }
        }
    }
    modules
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join("deka.lock").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

struct ResolvedImportTarget {
    module_id: String,
    file_path: PathBuf,
    integrity_target: Option<PackageIntegrityTarget>,
}

struct PackageIntegrityTarget {
    name: String,
    package_root: PathBuf,
}

fn resolve_import_target(
    raw: &str,
    current_file_path: &str,
    modules_root: Option<&Path>,
    available_modules: Option<&HashSet<String>>,
) -> Result<ResolvedImportTarget, ValidationError> {
    let raw = raw.trim();
    let is_relative = raw.starts_with('.');
    let is_project_alias = raw.starts_with("@/");
    let spec_path = raw.strip_prefix("@/").unwrap_or(raw);
    let mut base_dirs: Vec<PathBuf> = Vec::new();
    if is_relative {
        if let Some(parent) = Path::new(current_file_path).parent() {
            base_dirs.push(parent.to_path_buf());
        }
    } else if is_project_alias {
        if let Some(project_root) = modules_root.and_then(|root| root.parent()) {
            base_dirs.push(project_root.to_path_buf());
        }
        if let Ok(root) = std::env::var("DEKA_MODULE_ROOT") {
            let root = root.trim();
            if !root.is_empty() {
                base_dirs.push(PathBuf::from(root));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            base_dirs.push(cwd);
        }
    } else {
        if let Some(root) = modules_root {
            base_dirs.push(root.to_path_buf());
        }
    }

    if base_dirs.is_empty() {
        let lock_status = describe_lock_status(current_file_path);
        return Err(module_error(
            1,
            1,
            raw.len().max(1),
            format!(
                "Missing ds_modules for import '{}' in {} ({lock_status}).",
                raw, current_file_path
            ),
            "Create ds_modules/, ensure deka.lock is present, or set DEKA_MODULE_ROOT to a root that contains deka.lock.",
        ));
    }

    // Build the set of spec variants to try. For bare stdlib-style specifiers
    // we also try the @deka-scoped layout because stdlib packages installed
    // via `deka install` live under ds_modules/@deka/<pkg>/<subpath>.
    let mut spec_variants: Vec<String> = vec![spec_path.to_string()];
    if !is_relative && !is_project_alias && !raw.starts_with('@') && !raw.is_empty() {
        spec_variants.push(format!("@deka/{}", spec_path));
    }
    if !is_relative && !is_project_alias {
        if let Some(rest) = raw.strip_prefix("@deka/") {
            if !rest.is_empty() {
                spec_variants.push(rest.to_string());
            }
        }
    }

    let mut candidates = Vec::new();
    for base_dir in &base_dirs {
        for variant in &spec_variants {
            let base_path = base_dir.join(variant);
            if raw.ends_with(".ds") {
                candidates.push(base_path.clone());
                candidates.push(base_path.join("index.ds"));
            } else {
                // Extensionless relative/project specifiers resolve to .ds only.
                let ds_file_candidate = base_path.with_extension("ds");
                let ds_index_candidate = base_path.join("index.ds");
                if ds_file_candidate.exists() && ds_index_candidate.exists() {
                    return Err(module_error(
                        1,
                        1,
                        raw.len().max(1),
                        format!(
                            "Ambiguous import '{}' (both '{}' and '{}' exist).",
                            raw,
                            ds_file_candidate.display(),
                            ds_index_candidate.display()
                        ),
                        "Disambiguate the import by using an explicit path ending in .ds.",
                    ));
                }
                candidates.push(ds_file_candidate);
                candidates.push(ds_index_candidate);
            }
        }
    }
    if !is_relative && !is_project_alias {
        if let Some(root) = modules_root {
            for variant in &spec_variants {
                candidates.push(root.join(format!("{variant}.ds")));
                candidates.push(root.join(variant).join("index.ds"));
            }
        }
    }

    for candidate in candidates {
        if candidate.exists() {
            if is_project_alias {
                for project_root in &base_dirs {
                    if let Ok(rel) = candidate.strip_prefix(project_root) {
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        let module_id = format!("@/{}", module_id_from_rel(&rel));
                        return Ok(ResolvedImportTarget {
                            module_id,
                            file_path: candidate,
                            integrity_target: None,
                        });
                    }
                }
            } else if let Some(root) = modules_root {
                if let Ok(rel) = candidate.strip_prefix(root) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    let module_id = module_id_from_rel(&rel);
                    let integrity_target = package_integrity_target(raw, root, &candidate, &rel);
                    return Ok(ResolvedImportTarget {
                        module_id,
                        file_path: candidate,
                        integrity_target,
                    });
                }
            }
            return Ok(ResolvedImportTarget {
                module_id: raw.to_string(),
                file_path: candidate,
                integrity_target: None,
            });
        }
    }

    let attempted_roots = base_dirs
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let lock_status = describe_lock_status(current_file_path);
    let help = if is_relative {
        format!(
            "Ensure the module file exists relative to '{}'. Tried .ds extensions.",
            current_file_path
        )
    } else {
        available_modules
            .and_then(|modules| format_available_modules(modules, "Available modules: "))
            .unwrap_or_else(|| {
                "Ensure the module exists in ds_modules and is listed in deka.lock.".to_string()
            })
    };
    Err(module_error(
        1,
        1,
        raw.len().max(1),
        format!(
            "Missing module '{}' (imported from {}). Attempted roots: {}. {}",
            raw,
            current_file_path,
            if attempted_roots.is_empty() {
                "<none>"
            } else {
                attempted_roots.as_str()
            },
            lock_status
        ),
        help.as_str(),
    ))
}

fn package_integrity_target(
    raw: &str,
    modules_root: &Path,
    candidate: &Path,
    rel: &str,
) -> Option<PackageIntegrityTarget> {
    let rel_module_id = module_id_from_rel(rel);
    let name =
        package_name_from_import(raw).or_else(|| deka_stdlib_package_from_rel(&rel_module_id))?;
    let package_root = package_root_for_module(&name, modules_root, candidate, &rel_module_id)?;
    Some(PackageIntegrityTarget { name, package_root })
}

fn package_name_from_import(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with("@deka/") {
        let mut parts = trimmed.split('/').filter(|part| !part.is_empty());
        let scope = parts.next()?;
        let name = parts.next()?;
        return Some(format!("{}/{}", scope, name));
    }
    if trimmed.starts_with('@') && !trimmed.starts_with("@/") {
        return package_name_from_module_id(trimmed);
    }
    None
}

fn deka_stdlib_package_from_rel(module_id: &str) -> Option<String> {
    let mut parts = module_id.split('/').filter(|part| !part.is_empty());
    let first = parts.next()?;
    if first == "deka" {
        let second = parts.next()?;
        if second == "vault" {
            return Some("@deka/vault".to_string());
        }
        return None;
    }
    if is_deka_stdlib_root(first) {
        return Some(format!("@deka/{}", first));
    }
    None
}

fn is_deka_stdlib_root(root: &str) -> bool {
    matches!(
        root,
        "array"
            | "auth"
            | "buffer"
            | "bytes"
            | "component"
            | "core"
            | "cookies"
            | "crypto"
            | "db"
            | "encoding"
            | "fs"
            | "http"
            | "json"
            | "jwt"
            | "neo4j"
            | "payments"
            | "redis"
            | "string"
            | "tcp"
            | "time"
            | "tls"
    )
}

fn package_root_for_module(
    package_name: &str,
    modules_root: &Path,
    candidate: &Path,
    module_id: &str,
) -> Option<PathBuf> {
    if package_name.starts_with("@deka/") {
        let mut parts = module_id.split('/').filter(|part| !part.is_empty());
        let first = parts.next()?;
        if first == "@deka" {
            let name = parts.next()?;
            return Some(modules_root.join("@deka").join(name));
        }
        if first == "deka" && parts.next() == Some("vault") {
            return Some(modules_root.join("deka").join("vault"));
        }
        return Some(modules_root.join(first));
    }

    let mut parts = package_name.split('/').filter(|part| !part.is_empty());
    let scope = parts.next()?;
    let name = parts.next()?;
    let scoped_root = modules_root.join(scope).join(name);
    if candidate.starts_with(&scoped_root) {
        return Some(scoped_root);
    }
    None
}

struct ResolvedWasmTarget {
    root_path: PathBuf,
    manifest_path: PathBuf,
}

fn resolve_wasm_target(
    raw: &str,
    current_file_path: &str,
    modules_root: Option<&Path>,
    available_wasm: Option<&HashSet<String>>,
) -> Result<ResolvedWasmTarget, ValidationError> {
    let raw = raw.trim();
    let modules_root = modules_root.ok_or_else(|| {
        let lock_status = describe_lock_status(current_file_path);
        wasm_error(
            1,
            1,
            raw.len().max(1),
            format!(
                "Wasm import requires ds_modules/ (missing for {}, {}).",
                current_file_path, lock_status
            ),
            "Create ds_modules/, ensure deka.lock is present, or set DEKA_MODULE_ROOT to a root with deka.lock.",
        )
    })?;

    let is_relative = raw.starts_with('.');
    let is_project_alias = raw.starts_with("@/");
    let spec_path = raw.strip_prefix("@/").unwrap_or(raw);
    let base_dir = if is_relative {
        Path::new(current_file_path)
            .parent()
            .unwrap_or(modules_root)
            .to_path_buf()
    } else if is_project_alias {
        modules_root.parent().unwrap_or(modules_root).to_path_buf()
    } else {
        modules_root.to_path_buf()
    };
    let root_path = base_dir.join(spec_path);
    let allowed_root = if is_project_alias {
        modules_root.parent().unwrap_or(modules_root)
    } else {
        modules_root
    };
    let rel = root_path
        .strip_prefix(allowed_root)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"));
    if rel.as_deref().unwrap_or("").starts_with("..") || rel.is_none() {
        return Err(wasm_error(
            1,
            1,
            raw.len().max(1),
            format!(
                "Wasm import must resolve inside {} ({}: {}).",
                if is_project_alias {
                    "project root"
                } else {
                    "ds_modules/"
                },
                current_file_path,
                raw
            ),
            if is_project_alias {
                "Move the wasm module under the project root."
            } else {
                "Move the wasm module under ds_modules/."
            },
        ));
    }

    let manifest_path = root_path.join("deka.json");
    if !manifest_path.exists() {
        let help = available_wasm
            .and_then(|modules| format_available_modules(modules, "Available WASM modules: "));
        return Err(wasm_error(
            1,
            1,
            raw.len().max(1),
            format!(
                "Missing wasm module manifest for '{}' (expected {}).",
                raw,
                manifest_path.display()
            ),
            help.as_deref()
                .unwrap_or("Add deka.json to the wasm module directory."),
        ));
    }

    Ok(ResolvedWasmTarget {
        root_path,
        manifest_path,
    })
}

fn validate_wasm_manifest(
    target: &ResolvedWasmTarget,
    spec: &ImportSpec,
    errors: &mut Vec<ValidationError>,
) {
    let raw = match std::fs::read_to_string(&target.manifest_path) {
        Ok(raw) => raw,
        Err(err) => {
            errors.push(wasm_error(
                spec.line,
                spec.column,
                spec.from.len().max(1),
                format!(
                    "Failed to read wasm manifest {}: {}",
                    target.manifest_path.display(),
                    err
                ),
                "Ensure the manifest is readable JSON.",
            ));
            return;
        }
    };

    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            errors.push(wasm_error(
                spec.line,
                spec.column,
                spec.from.len().max(1),
                format!(
                    "Invalid wasm manifest {}: {}",
                    target.manifest_path.display(),
                    err
                ),
                "Fix the JSON in deka.json.",
            ));
            return;
        }
    };
    let module_path = parsed
        .get("module")
        .and_then(|v| v.as_str())
        .unwrap_or("module.wasm");
    let module_path = target.root_path.join(module_path);
    if !module_path.exists() {
        errors.push(wasm_error(
            spec.line,
            spec.column,
            spec.from.len().max(1),
            format!("Missing wasm module binary {}.", module_path.display()),
            "Build the wasm module or update deka.json.",
        ));
    }

    let stub_path = parsed
        .get("stubs")
        .and_then(|v| v.as_str())
        .map(|s| target.root_path.join(s))
        .unwrap_or_else(|| target.root_path.join("module.d.phpx"));
    if !stub_path.exists() {
        errors.push(wasm_error(
            spec.line,
            spec.column,
            spec.from.len().max(1),
            format!("Missing wasm stub file {}.", stub_path.display()),
            "Generate stubs with `deka wasm stubs`.",
        ));
        return;
    }

    let stub_source = match std::fs::read_to_string(&stub_path) {
        Ok(src) => src,
        Err(err) => {
            errors.push(wasm_error(
                spec.line,
                spec.column,
                spec.from.len().max(1),
                format!("Failed to read wasm stub {}: {}", stub_path.display(), err),
                "Ensure the stub file is readable.",
            ));
            return;
        }
    };
    let exports = collect_exports(&stub_source, stub_path.to_string_lossy().as_ref());
    if !exports.contains(&spec.imported) {
        errors.push(wasm_error(
            spec.line,
            spec.column,
            spec.imported.len().max(1),
            format!(
                "Missing wasm export '{}' in {}.",
                spec.imported,
                stub_path.display()
            ),
            "Regenerate stubs or update the import to match exported names.",
        ));
    }
}

fn module_id_from_rel(rel: &str) -> String {
    let normalized = rel.replace('\\', "/");
    if normalized.ends_with("/index.phpx") {
        return normalized.replace("/index.phpx", "");
    }
    normalized.trim_end_matches(".phpx").to_string()
}

fn is_valid_user_module(raw: &str) -> bool {
    if !raw.starts_with('@') {
        return true;
    }
    let parts: Vec<&str> = raw.split('/').collect();
    parts.len() == 2
        && parts[0].len() > 1
        && !parts[1].is_empty()
        && parts.iter().all(|part| !part.trim().is_empty())
}

fn adwa_capability_block(specifier: &str) -> Option<(&'static str, &'static str, &'static str)> {
    if specifier == "db"
        || specifier.starts_with("db/")
        || specifier == "postgres"
        || specifier.starts_with("postgres/")
        || specifier == "mysql"
        || specifier.starts_with("mysql/")
        || specifier == "sqlite"
        || specifier.starts_with("sqlite/")
    {
        return Some((
            "db",
            "database host capability is disabled",
            "Move db access behind a server endpoint for adwa.",
        ));
    }

    if specifier == "process"
        || specifier.starts_with("process/")
        || specifier == "env"
        || specifier.starts_with("env/")
    {
        return Some((
            "process/env",
            "process/env host capability is disabled",
            "Inject config through app context instead of reading process/env in adwa.",
        ));
    }

    None
}

fn format_available_modules(modules: &HashSet<String>, prefix: &str) -> Option<String> {
    if modules.is_empty() {
        return None;
    }
    let mut list: Vec<String> = modules.iter().cloned().collect();
    list.sort();
    let preview: Vec<String> = list.into_iter().take(12).collect();
    Some(format!("{}{}", prefix, preview.join(", ")))
}

fn describe_lock_status(current_file_path: &str) -> String {
    let path = Path::new(current_file_path);
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let local = find_project_root(&dir)
        .map(|root| format!("local lock: {}", root.join("deka.lock").display()))
        .unwrap_or_else(|| "local lock: not found".to_string());
    let global = match std::env::var("DEKA_MODULE_ROOT") {
        Ok(root) if !root.trim().is_empty() => {
            let lock = PathBuf::from(root.trim()).join("deka.lock");
            if lock.exists() {
                format!("global lock: {}", lock.display())
            } else {
                format!("global lock: missing at {}", lock.display())
            }
        }
        _ => "global lock: DEKA_MODULE_ROOT unset".to_string(),
    };
    format!("{local}; {global}")
}

fn validate_package_integrity(
    modules_root: &Path,
    package_roots: &HashMap<String, PathBuf>,
) -> Vec<ValidationError> {
    if package_roots.is_empty() {
        return Vec::new();
    }

    let lock_path = modules_root.parent().map(|root| root.join("deka.lock"));
    let Some(lock_path) = lock_path else {
        return package_roots
            .keys()
            .map(|name| {
                module_error(
                    1,
                    1,
                    name.len().max(1),
                    format!(
                        "Missing deka.lock; cannot verify integrity for package '{}'.",
                        name
                    ),
                    "Create or restore deka.lock before running third-party modules.",
                )
            })
            .collect();
    };

    let lock_raw = match std::fs::read_to_string(&lock_path) {
        Ok(raw) => raw,
        Err(err) => {
            return package_roots
                .keys()
                .map(|name| {
                    module_error(
                        1,
                        1,
                        name.len().max(1),
                        format!(
                            "Failed to read {} for integrity validation: {}",
                            lock_path.display(),
                            err
                        ),
                        "Fix file permissions or restore the lockfile.",
                    )
                })
                .collect();
        }
    };

    let lock_json: Value = match serde_json::from_str(&lock_raw) {
        Ok(json) => json,
        Err(err) => {
            return package_roots
                .keys()
                .map(|name| {
                    module_error(
                        1,
                        1,
                        name.len().max(1),
                        format!(
                            "Invalid deka.lock JSON; cannot verify integrity for '{}': {}",
                            name, err
                        ),
                        "Recreate the lockfile with a clean install.",
                    )
                })
                .collect();
        }
    };

    let packages_json = lock_json
        .get("packages")
        .and_then(|value| value.as_object())
        .or_else(|| {
            lock_json
                .pointer("/php/packages")
                .and_then(|value| value.as_object())
        });
    let Some(packages_json) = packages_json else {
        return package_roots
            .keys()
            .map(|name| {
                module_error(
                    1,
                    1,
                    name.len().max(1),
                    format!(
                        "deka.lock has no package entries; cannot verify '{}'.",
                        name
                    ),
                    "Run `deka install` to recreate package entries.",
                )
            })
            .collect();
    };

    let mut cache: HashMap<String, (String, String)> = HashMap::new();
    let mut errors = Vec::new();
    for (name, package_root) in package_roots {
        let entry = match packages_json.get(name) {
            Some(value) => value,
            None => {
                if name.starts_with("@user/") {
                    continue;
                }
                errors.push(module_error(
                    1,
                    1,
                    name.len().max(1),
                    format!("Package '{}' is missing from deka.lock.", name),
                    "Reinstall the package to regenerate the lock entry.",
                ));
                continue;
            }
        };

        let metadata = entry
            .as_array()
            .and_then(|items| items.get(2))
            .and_then(|value| value.as_object());
        let Some(metadata) = metadata else {
            errors.push(module_error(
                1,
                1,
                name.len().max(1),
                format!("Package '{}' lock entry is malformed.", name),
                "Reinstall the package to regenerate the lock entry.",
            ));
            continue;
        };

        let expected_module_graph = metadata
            .get("moduleGraph")
            .and_then(|value| value.get("hash"))
            .and_then(|value| value.as_str());
        let expected_fs_graph = metadata
            .get("fsGraph")
            .and_then(|value| value.get("hash"))
            .and_then(|value| value.as_str());

        if expected_module_graph.is_none() || expected_fs_graph.is_none() {
            errors.push(module_error(
                1,
                1,
                name.len().max(1),
                format!("Package '{}' lock entry is missing integrity hashes.", name),
                "Reinstall the package to regenerate integrity hashes.",
            ));
            continue;
        }

        let (module_hash, fs_hash) = match cache.get(name) {
            Some(values) => values.clone(),
            None => match compute_package_integrity(&package_root) {
                Ok(integrity) => {
                    let values = (integrity.module_graph, integrity.fs_graph);
                    cache.insert(name.clone(), values.clone());
                    values
                }
                Err(err) => {
                    errors.push(module_error(
                        1,
                        1,
                        name.len().max(1),
                        format!("Failed to compute integrity for '{}': {}", name, err),
                        "Ensure the package directory exists and is readable.",
                    ));
                    continue;
                }
            },
        };

        if expected_module_graph != Some(module_hash.as_str())
            || expected_fs_graph != Some(fs_hash.as_str())
        {
            errors.push(module_error(
                1,
                1,
                name.len().max(1),
                format!(
                    "Package '{}' failed integrity check (lockfile mismatch).",
                    name
                ),
                "Reinstall the package to restore the expected content.",
            ));
        }
    }

    errors
}

fn package_name_from_module_id(module_id: &str) -> Option<String> {
    let trimmed = module_id.trim();
    if trimmed.is_empty() || trimmed.starts_with("@/") {
        return None;
    }

    if trimmed.starts_with('@') {
        let mut parts = trimmed.split('/').filter(|part| !part.is_empty());
        let scope = parts.next()?;
        if scope == "@/" {
            return None;
        }
        let name = parts.next()?;
        return Some(format!("{}/{}", scope, name));
    }

    None
}

fn module_error(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
) -> ValidationError {
    ValidationError {
        kind: ErrorKind::ModuleError,
        line,
        column,
        message,
        help_text: help_text.to_string(),
        suggestion: None,
        underline_length: underline_length.max(1),
        severity: Severity::Error,
    }
}

fn wasm_error(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
) -> ValidationError {
    ValidationError {
        kind: ErrorKind::WasmError,
        line,
        column,
        message,
        help_text: help_text.to_string(),
        suggestion: None,
        underline_length: underline_length.max(1),
        severity: Severity::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_modules_root_with_env, validate_module_resolution, validate_package_integrity,
        validate_target_capabilities, MODULES_DIR,
    };
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_project(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("deka_modules_test_{name}_{nanos}"));
        fs::create_dir_all(root.join(MODULES_DIR)).expect("create php_modules");
        fs::write(root.join("deka.lock"), "{}").expect("write lockfile");
        root
    }

    fn make_temp_modules_root(name: &str, with_lock: bool) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("deka_modules_env_{name}_{nanos}"));
        fs::create_dir_all(root.join(MODULES_DIR)).expect("create php_modules");
        if with_lock {
            fs::write(root.join("deka.lock"), "{}").expect("write lockfile");
        }
        root
    }

    fn write_lock_for_packages(root: &std::path::Path, packages: &[(&str, &str)]) {
        let mut entries = serde_json::Map::new();
        for (name, rel_root) in packages {
            let package_root = root.join(MODULES_DIR).join(rel_root);
            let integrity =
                crate::integrity::compute_package_integrity(&package_root).expect("integrity");
            entries.insert(
                (*name).to_string(),
                serde_json::json!([
                    format!("{}@0.1.0", name),
                    format!("linkhash:{}", name),
                    {
                        "moduleGraph": { "hash": integrity.module_graph },
                        "fsGraph": { "hash": integrity.fs_graph }
                    },
                    ""
                ]),
            );
        }
        fs::write(
            root.join("deka.lock"),
            serde_json::json!({
                "lockfileVersion": 1,
                "packages": entries
            })
            .to_string(),
        )
        .expect("write lock");
    }

    #[test]
    fn resolve_modules_root_prefers_local_project_over_phpx_module_root() {
        let local = make_temp_project("local_precedence");
        let global = make_temp_modules_root("global_precedence", true);
        let entry = local.join("app").join("main.phpx");
        fs::create_dir_all(entry.parent().expect("entry parent")).expect("mkdir app");
        fs::write(&entry, "import { foo } from 'a'\n").expect("write entry");

        let resolved = resolve_modules_root_with_env(
            entry.to_string_lossy().as_ref(),
            Some(global.to_string_lossy().as_ref()),
        )
        .expect("resolve modules root");

        assert_eq!(resolved, local.join(MODULES_DIR));
        let _ = fs::remove_dir_all(local);
        let _ = fs::remove_dir_all(global);
    }

    #[test]
    fn resolve_modules_root_supports_global_only_mode() {
        let global = make_temp_modules_root("global_only", true);
        let outside = std::env::temp_dir()
            .join("deka_no_local_project")
            .join("entry.phpx");
        fs::create_dir_all(outside.parent().expect("outside parent")).expect("mkdir outside");
        fs::write(&outside, "import { foo } from 'a'\n").expect("write outside entry");

        let resolved = resolve_modules_root_with_env(
            outside.to_string_lossy().as_ref(),
            Some(global.to_string_lossy().as_ref()),
        )
        .expect("resolve modules root");

        assert_eq!(resolved, global.join(MODULES_DIR));
        let _ = fs::remove_dir_all(global);
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn global_only_mode_requires_global_lockfile() {
        let global = make_temp_modules_root("global_missing_lock", false);
        let outside = std::env::temp_dir()
            .join("deka_missing_lock_project")
            .join("entry.phpx");
        fs::create_dir_all(outside.parent().expect("outside parent")).expect("mkdir outside");
        fs::write(&outside, "import { foo } from 'a'\n").expect("write outside entry");

        let resolved = resolve_modules_root_with_env(
            outside.to_string_lossy().as_ref(),
            Some(global.to_string_lossy().as_ref()),
        );

        assert!(
            resolved.is_none(),
            "expected no modules root without global lock"
        );
        let _ = fs::remove_dir_all(global);
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn detects_plain_module_cycles() {
        let root = make_temp_project("plain_cycle");
        let entry = root.join("main.phpx");
        fs::write(&entry, "import { foo } from 'a'\n").expect("write entry");
        fs::write(
            root.join(MODULES_DIR).join("a.phpx"),
            "import { bar } from 'b'\nexport function foo() { return 1 }\n",
        )
        .expect("write a");
        fs::write(
            root.join(MODULES_DIR).join("b.phpx"),
            "import { foo } from 'a'\nexport function bar() { return 1 }\n",
        )
        .expect("write b");

        let errors = validate_module_resolution(
            &fs::read_to_string(&entry).expect("read entry"),
            entry.to_string_lossy().as_ref(),
        );
        assert!(
            errors
                .iter()
                .any(|err| err.message.contains("Cyclic phpx import detected:")),
            "expected plain cycle error, got: {:?}",
            errors
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reports_missing_module_with_actionable_message() {
        let root = make_temp_project("missing_module");
        let entry = root.join("main.phpx");
        fs::write(&entry, "import { foo } from 'does_not_exist'\n").expect("write entry");

        let errors = validate_module_resolution(
            &fs::read_to_string(&entry).expect("read entry"),
            entry.to_string_lossy().as_ref(),
        );
        assert!(
            errors
                .iter()
                .any(|err| err.message.contains("Missing module 'does_not_exist'")),
            "expected missing module error, got: {:?}",
            errors
        );
        assert!(
            errors.iter().any(|err| err
                .help_text
                .contains("Ensure the module exists in ds_modules")
                || err.help_text.contains("Available modules:")),
            "expected actionable help text, got: {:?}",
            errors
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_relative_ds_imports_in_same_directory() {
        let root = make_temp_project("relative_ds_import");
        let entry = root.join("main.ds");
        fs::write(&entry, "import { PI } from \"./constants.ds\"\nconsole.log(PI)\n")
            .expect("write entry");
        fs::write(root.join("constants.ds"), "export const PI = 3.14159\n")
            .expect("write constants");

        let errors = validate_module_resolution(
            &fs::read_to_string(&entry).expect("read entry"),
            entry.to_string_lossy().as_ref(),
        );
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reports_missing_relative_ds_module() {
        let root = make_temp_project("missing_relative_ds");
        let entry = root.join("main.ds");
        fs::write(&entry, "import { PI } from \"./constants.ds\"\n")
            .expect("write entry");

        let errors = validate_module_resolution(
            &fs::read_to_string(&entry).expect("read entry"),
            entry.to_string_lossy().as_ref(),
        );
        assert!(
            errors.iter().any(|err| err.message.contains("Missing module './constants.ds'")),
            "expected missing module error, got: {:?}",
            errors
        );
        assert!(
            errors.iter().any(|err| err.help_text.contains("Tried .ds and .phpx extensions")),
            "expected .ds/.phpx help text, got: {:?}",
            errors
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_scoped_deka_stdlib_imports_from_unscoped_install_dirs() {
        let root = make_temp_project("scoped_stdlib_unscoped_dir");
        let entry = root.join("main.phpx");
        fs::write(
            &entry,
            "\
import { http_get } from '@deka/http'
import { random_hex } from '@deka/crypto'
import { now_ms } from '@deka/time'
",
        )
        .expect("write entry");
        for (module, source) in [
            (
                "http",
                "export function http_get($url: string): object { return {} }\n",
            ),
            (
                "crypto",
                "export function random_hex($len: int = 16): string { return '00' }\n",
            ),
            ("time", "export function now_ms(): int { return 1 }\n"),
        ] {
            fs::create_dir_all(root.join(MODULES_DIR).join(module))
                .unwrap_or_else(|err| panic!("mkdir {module}: {err}"));
            fs::write(
                root.join(MODULES_DIR).join(module).join("index.phpx"),
                source,
            )
            .unwrap_or_else(|err| panic!("write {module}: {err}"));
        }
        write_lock_for_packages(
            &root,
            &[
                ("@deka/http", "http"),
                ("@deka/crypto", "crypto"),
                ("@deka/time", "time"),
            ],
        );

        let errors = validate_module_resolution(
            &fs::read_to_string(&entry).expect("read entry"),
            entry.to_string_lossy().as_ref(),
        );
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn scoped_deka_import_resolved_unscoped_still_checks_lock_integrity() {
        let root = make_temp_project("scoped_stdlib_unscoped_integrity");
        let entry = root.join("main.phpx");
        fs::write(
            &entry,
            "import { http_get } from '@deka/http'\nexport function run() { return http_get }\n",
        )
        .expect("write entry");
        let package_root = root.join(MODULES_DIR).join("http");
        fs::create_dir_all(&package_root).expect("mkdir http");
        fs::write(
            package_root.join("index.phpx"),
            "export function http_get($url: string): object { return {} }\n",
        )
        .expect("write http");
        write_lock_for_packages(&root, &[("@deka/http", "http")]);
        fs::write(
            package_root.join("index.phpx"),
            "export function http_get($url: string): object { return { tampered: true } }\n",
        )
        .expect("tamper http");

        let errors = validate_module_resolution(
            &fs::read_to_string(&entry).expect("read entry"),
            entry.to_string_lossy().as_ref(),
        );
        assert!(
            errors.iter().any(|err| err
                .message
                .contains("Package '@deka/http' failed integrity check")),
            "expected @deka/http integrity error, got: {:?}",
            errors
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reports_ambiguous_shorthand_vs_index_module() {
        let root = make_temp_project("ambiguous_module");
        let entry = root.join("main.phpx");
        fs::write(&entry, "import { foo } from 'ui/card'\n").expect("write entry");
        fs::create_dir_all(root.join(MODULES_DIR).join("ui/card")).expect("mkdir ui/card");
        fs::write(
            root.join(MODULES_DIR).join("ui/card.phpx"),
            "export function foo() { return 1 }\n",
        )
        .expect("write card.phpx");
        fs::write(
            root.join(MODULES_DIR).join("ui/card/index.phpx"),
            "export function foo() { return 2 }\n",
        )
        .expect("write card/index.phpx");

        let errors = validate_module_resolution(
            &fs::read_to_string(&entry).expect("read entry"),
            entry.to_string_lossy().as_ref(),
        );
        assert!(
            errors
                .iter()
                .any(|err| err.message.contains("Ambiguous import 'ui/card'")),
            "expected ambiguous import error, got: {:?}",
            errors
        );
        assert!(
            errors
                .iter()
                .any(|err| err.help_text.contains("explicit path ending in .phpx")),
            "expected remediation hint, got: {:?}",
            errors
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_integrity_accepts_flat_lock_packages() {
        let root = make_temp_project("flat_lock_integrity");
        let package_root = root.join(MODULES_DIR).join("@deka").join("core");
        fs::create_dir_all(&package_root).expect("mkdir package");
        fs::write(
            package_root.join("index.phpx"),
            "export function ok(): int { return 1 }\n",
        )
        .expect("write package");
        let integrity =
            crate::integrity::compute_package_integrity(&package_root).expect("integrity");
        fs::write(
            root.join("deka.lock"),
            serde_json::json!({
                "lockfileVersion": 1,
                "packages": {
                    "@deka/core": [
                        "0.1.0",
                        "linkhash:@deka/core",
                        {
                            "moduleGraph": { "hash": integrity.module_graph },
                            "fsGraph": { "hash": integrity.fs_graph }
                        },
                        ""
                    ]
                }
            })
            .to_string(),
        )
        .expect("write lock");

        let package_roots = HashMap::from([("@deka/core".to_string(), package_root)]);
        let errors = validate_package_integrity(&root.join(MODULES_DIR), &package_roots);
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_top_level_await_cycles_with_path() {
        let root = make_temp_project("tla_cycle");
        let entry = root.join("main.phpx");
        fs::write(&entry, "import { foo } from 'a'\n").expect("write entry");
        fs::write(
            root.join(MODULES_DIR).join("a.phpx"),
            "import { bar } from 'b'\nexport function foo() { return 1 }\n",
        )
        .expect("write a");
        fs::write(
            root.join(MODULES_DIR).join("b.phpx"),
            "import { foo } from 'a'\n$v = await foo()\nexport function bar() { return 1 }\n",
        )
        .expect("write b");

        let errors = validate_module_resolution(
            &fs::read_to_string(&entry).expect("read entry"),
            entry.to_string_lossy().as_ref(),
        );
        assert!(
            errors.iter().any(|err| err
                .message
                .contains("Top-level await import cycle detected:")),
            "expected top-level await cycle error, got: {:?}",
            errors
        );
        assert!(
            errors.iter().any(|err| err.message.contains("->")),
            "expected cycle path details, got: {:?}",
            errors
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn blocks_db_imports_for_adwa_target() {
        // SAFETY: test process controls env mutations in this isolated test.
        unsafe {
            std::env::set_var("DEKA_TARGET", "adwa");
        }
        let source = "import { query } from 'db/postgres'\n";
        let errors = validate_target_capabilities(source, "main.phpx");
        // SAFETY: test process controls env mutations in this isolated test.
        unsafe {
            std::env::remove_var("DEKA_TARGET");
        }
        assert_eq!(
            errors.len(),
            1,
            "expected one capability error: {:?}",
            errors
        );
        assert!(
            errors[0].message.contains("db/postgres"),
            "expected module in message: {:?}",
            errors
        );
    }

    #[test]
    fn allows_db_imports_for_server_target() {
        // SAFETY: test process controls env mutations in this isolated test.
        unsafe {
            std::env::remove_var("DEKA_TARGET");
            std::env::set_var("DEKA_HOST_PROFILE", "server");
        }
        let source = "import { query } from 'db/postgres'\n";
        let errors = validate_target_capabilities(source, "main.phpx");
        // SAFETY: test process controls env mutations in this isolated test.
        unsafe {
            std::env::remove_var("DEKA_HOST_PROFILE");
        }
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }
}
