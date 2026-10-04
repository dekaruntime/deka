//! DSC is linked as a library; no JS emission or compiler subprocess.
use crate::{Function, Hosts, ListMut, Literal, Op, Program, PromiseJoin, Result};
pub use deka_syntax::console::STDERR_OPERATION as CONSOLE_ERROR_OPERATION;
use deka_syntax::{Diagnostic, Severity, ast::*, typeck::ExceptionEmit};
use std::collections::{BTreeMap, HashMap};
#[path = "builtin_modules.rs"]
mod builtins;
#[path = "json_compiler.rs"]
mod json_lower;

pub fn compile(source: &str, hosts: &Hosts) -> Result<Program> {
    compile_entry(source, hosts, "main")
}
/// Compile an in-memory module with a required entry function.
pub fn compile_entry(source: &str, hosts: &Hosts, entry_name: &str) -> Result<Program> {
    compile_modules(
        &[(std::path::PathBuf::from("<source>"), source.to_owned())],
        hosts,
        Some(entry_name),
        &Project {
            root: None,
            dependencies: BTreeMap::new(),
            lock: BTreeMap::new(),
        },
        None,
    )
}
/// Compile a source file and its relative modules, once each, in dependency order.
/// Import cycles load with JavaScript module semantics (deka#1206): a module
/// already being loaded is not loaded again, and reading one of its exports
/// before it finishes initializing is a named error, not a silent value.
/// Self-imports and external packages fail explicitly.
pub fn compile_file(path: &std::path::Path, hosts: &Hosts, entry: Option<&str>) -> Result<Program> {
    let project = Project::load(path)?;
    compile_modules(&load_modules(path, &project)?, hosts, entry, &project, None)
}
/// Native source graph and public names needed to check a local package through
/// an installed consumer. Uses the same resolver and export collector as compile.
pub struct PackageCheckInputs {
    pub sources: Vec<(std::path::PathBuf, String)>,
    pub exports: Vec<PackageExport>,
}
pub struct PackageExport {
    pub name: String,
    pub type_only: bool,
}
/// Declared type exports have checker metadata, not ordinary runtime cells.
/// A same-named value, when present, still needs its actual exported binding.
fn erased_export(surface: &deka_syntax::ModuleExports<'_>, name: &str) -> bool {
    !surface.values.contains_key(name)
        && (surface.interfaces.contains_key(name)
            || surface.structs.contains_key(name)
            || surface.enums.contains_key(name)
            || surface.aliases.contains_key(name)
            || surface.opaques.contains_key(name)
            || surface.newtypes.contains_key(name))
}
pub fn package_check_inputs(
    path: &std::path::Path,
    name: &str,
    hosts: &Hosts,
) -> Result<PackageCheckInputs> {
    if host_module(name) {
        return Err(format!(
            "package {name} resolves to a native builtin, not a ds_modules package"
        ));
    }
    let project = Project::load(path)?;
    let sources = load_modules(path, &project)?;
    let mut exports = Vec::new();
    compile_modules(&sources, hosts, None, &project, Some(&mut exports))?;
    Ok(PackageCheckInputs { sources, exports })
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
                let source = match stmt {
                    Stmt::Import { source, .. } => Some(*source),
                    Stmt::Export {
                        decl:
                            ExportDecl::NamedGroup {
                                source: Some(source),
                                ..
                            },
                        ..
                    } => Some(*source),
                    _ => None,
                };
                if let Some(source) = source
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
    builtins::contains(source)
}

/// The project a compile resolves packages against: the nearest ancestor of
/// the entry module with a `deka.json`, its declared dependency pins, and
/// the lock's exact pins (deka#1212). Consumption only — nothing here
/// touches the network. A program that imports no package needs no
/// `deka.json`: `root` is `None` then, and only resolving a bare specifier
/// reports the missing manifest.
pub struct Project {
    root: Option<std::path::PathBuf>,
    dependencies: BTreeMap<String, String>,
    lock: BTreeMap<String, String>,
}

impl Project {
    fn load(entry: &std::path::Path) -> Result<Project> {
        let mut directory = std::fs::canonicalize(entry)
            .map_err(|e| format!("{}: {e}", entry.display()))?
            .parent()
            .ok_or("module has no parent")?
            .to_path_buf();
        let root = loop {
            if directory.join("deka.json").exists() {
                break directory;
            }
            if !directory.pop() {
                return Ok(Project {
                    root: None,
                    dependencies: BTreeMap::new(),
                    lock: BTreeMap::new(),
                });
            }
        };
        let manifest: serde_json::Value = std::fs::read_to_string(root.join("deka.json"))
            .ok()
            .and_then(|bytes| serde_json::from_str(&bytes).ok())
            .ok_or_else(|| format!("{}: invalid deka.json", root.display()))?;
        let dependencies = manifest
            .get("dependencies")
            .and_then(|d| d.as_object())
            .map(|deps| {
                deps.iter()
                    .filter_map(|(name, pin)| Some((name.clone(), pin.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let lock = std::fs::read_to_string(root.join("deka.lock"))
            .ok()
            .and_then(|bytes| serde_json::from_str::<crate::package::Lock>(&bytes).ok())
            .map(crate::package::Lock::pins)
            .unwrap_or_default();
        Ok(Project {
            root: Some(root),
            dependencies,
            lock,
        })
    }

    /// Resolve a bare specifier to a package file: `name` (or
    /// `@scope/name`) plus an optional subpath. Every failure names the
    /// package and the cause.
    fn package_path(&self, importer: &std::path::Path, source: &str) -> Result<std::path::PathBuf> {
        let segments: Vec<&str> = source.split('/').collect();
        let (name, subpath) = if source.starts_with('@') {
            if segments.len() < 2 {
                return Err(format!("invalid package specifier: {source}"));
            }
            (
                format!("{}/{}", segments[0], segments[1]),
                &segments[2.min(segments.len())..],
            )
        } else {
            (segments[0].to_owned(), &segments[1.min(segments.len())..])
        };
        let Some(root) = &self.root else {
            return Err(format!(
                "package {name} cannot be resolved: deka.json not found in this directory or any parent"
            ));
        };
        let mut dependencies = None;
        if let Ok(relative) = importer.strip_prefix(root.join("ds_modules")) {
            let mut components = relative.components();
            if let Some(std::path::Component::Normal(first)) = components.next() {
                let mut owner = root.join("ds_modules").join(first);
                if first.to_string_lossy().starts_with('@')
                    && let Some(std::path::Component::Normal(package)) = components.next()
                {
                    owner.push(package);
                }
                let manifest_path = owner.join("deka.json");
                let manifest: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(&manifest_path)
                        .map_err(|e| format!("{}: {e}", manifest_path.display()))?,
                )
                .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
                // Hand-placed legacy packages without this field used the root
                // declarations. An explicit package map supplies its own scope.
                if let Some(value) = manifest.get("dependencies") {
                    dependencies = Some(
                        serde_json::from_value::<BTreeMap<String, String>>(value.clone()).map_err(
                            |e| format!("{}: invalid dependencies: {e}", manifest_path.display()),
                        )?,
                    );
                }
            }
        }
        let declarations = dependencies.as_ref().unwrap_or(&self.dependencies);
        let Some(declared) = declarations.get(&name) else {
            return Err(format!(
                "package {name} is not declared in deka.json dependencies"
            ));
        };
        let directory = root.join("ds_modules").join(&name);
        if !directory.is_dir() {
            return Err(format!(
                "package {name} is not installed (ds_modules/{name} is missing)"
            ));
        };
        let installed_manifest = std::fs::read_to_string(directory.join("deka.json"))
            .ok()
            .and_then(|bytes| serde_json::from_str::<serde_json::Value>(&bytes).ok());
        // The importing manifest declares, deka.lock pins, ds_modules provides.
        // Check agreement for both root and subpath imports.
        if let Some(installed) = installed_manifest
            .as_ref()
            .and_then(|manifest| manifest.get("version")?.as_str())
        {
            if installed != declared {
                return Err(format!(
                    "package {name} version mismatch: deka.json expects {declared}, ds_modules has {installed}"
                ));
            }
            if let Some(pin) = self.lock.get(&name)
                && pin != installed
            {
                return Err(format!(
                    "package {name} version mismatch: deka.lock pins {pin}, ds_modules has {installed}"
                ));
            }
        }
        let entry = if subpath.is_empty() {
            let entry = installed_manifest
                .as_ref()
                .and_then(|manifest| manifest.get("entry")?.as_str())
                .unwrap_or("index.ds");
            directory.join(entry)
        } else {
            directory.join(subpath.join("/"))
        };
        if !entry.exists() {
            return Err(format!(
                "package {name} has no entry file ({})",
                entry.display()
            ));
        }
        std::fs::canonicalize(&entry).map_err(|e| format!("{}: {e}", entry.display()))
    }
}

fn module_path(
    parent: &std::path::Path,
    source: &str,
    project: &Project,
) -> Result<std::path::PathBuf> {
    if source.starts_with("./") || source.starts_with("../") {
        let path = parent.parent().ok_or("module has no parent")?.join(source);
        return std::fs::canonicalize(&path).map_err(|e| format!("{}: {e}", path.display()));
    }
    if host_module(source) {
        return Ok(builtins::path(source));
    }
    project.package_path(parent, source)
}
/// Stamp a refusal once at the statement that produced it. The graph stamps
/// only resolution failures on its own edges; recursive module errors already
/// carry the dependency's source and must pass through unchanged.
fn source_refusal(source_file: &str, span: Span, error: String) -> String {
    if error.starts_with(&format!("{source_file}: ")) {
        error
    } else {
        format!(
            "{source_file}: {}:{}: {error}",
            span.start.line, span.start.column
        )
    }
}

/// Modules in dependency order. A module already being loaded is not loaded
/// again (import cycles load, deka#1206); reads of exports that initialize
/// later than the importer become checked loads during lowering, computed
/// from the loaded order in `compile_modules`. Self-imports are refused.
fn load_modules(
    path: &std::path::Path,
    project: &Project,
) -> Result<Vec<(std::path::PathBuf, String)>> {
    fn visit(
        path: std::path::PathBuf,
        project: &Project,
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
            let source = match stmt {
                Stmt::Import { source, .. } if !host_module(source) => Some(*source),
                // `export { x } from "./y.ds"` is a load edge like an import.
                Stmt::Export {
                    decl:
                        ExportDecl::NamedGroup {
                            source: Some(source),
                            ..
                        },
                    ..
                } => Some(*source),
                _ => None,
            };
            if let Some(source) = source.filter(|source| !host_module(source)) {
                let target = module_path(&path, source, project).map_err(|error| {
                    source_refusal(&path.display().to_string(), stmt.span(), error)
                })?;
                if target == path {
                    return Err(source_refusal(
                        &path.display().to_string(),
                        stmt.span(),
                        format!("cyclic module import: {}", path.display()),
                    ));
                }
                if !visiting.contains(&target) {
                    visit(target, project, visiting, loaded)?;
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
        project,
        &mut Default::default(),
        &mut loaded,
    )?;
    Ok(loaded)
}
fn native_globals<'a>(
    hosts: &Hosts,
    host_exports: &deka_syntax::ModuleExports<'a>,
    arena: &'a bumpalo::Bump,
) -> HashMap<&'a str, deka_syntax::typeck::Type<'a>> {
    let mut globals: HashMap<_, _> = hosts
        .globals()
        .filter_map(|op| {
            host_exports
                .values
                .get_key_value(op.name.as_str())
                .map(|(_, ty)| {
                    let ty = if op.global_value.is_some() {
                        let deka_syntax::typeck::Type::Function { ret, .. } = ty else {
                            unreachable!("host getter declaration")
                        };
                        *ret.clone()
                    } else {
                        ty.clone()
                    };
                    (&*arena.alloc_str(op.global_name()), ty)
                })
        })
        .collect();
    for op in hosts.namespaces() {
        let (namespace, field) = op.namespace.as_ref().expect("host namespace");
        let name: &str = arena.alloc_str(namespace);
        let ty = host_exports
            .values
            .get(op.name.as_str())
            .expect("host declaration")
            .clone();
        let deka_syntax::typeck::Type::Object { fields } = globals
            .entry(name)
            .or_insert_with(|| deka_syntax::typeck::Type::Object { fields: vec![] })
        else {
            unreachable!("validated namespace/global collision");
        };
        fields.push((arena.alloc_str(field), ty));
    }
    let fields = PromiseJoin::ALL
        .into_iter()
        .map(|kind| {
            let name = format!("__promise_{}", kind.name());
            let mut ty = host_exports
                .values
                .get(name.as_str())
                .expect("native promise declaration")
                .clone();
            // Each intrinsic carries its own polymorphic binder through aliases.
            ty = deka_syntax::typeck::Type::Generic {
                base: if kind == PromiseJoin::All {
                    "$PromiseAll"
                } else {
                    "$PromiseRace"
                },
                args: vec![ty],
            };
            (kind.name(), ty)
        })
        .collect();
    globals.insert("Promise", deka_syntax::typeck::Type::Object { fields });
    globals
}

/// One module lowered into the shared entry function. Returns the module's
/// export-name → slot map. `forward` holds this module's cycle-closing
/// imports (local → (exported name, target)); in harvest mode
/// (`mark_checked = false`) those alias a placeholder local so body lowering
/// resolves, in the real pass they alias the exporter's slot and their reads
/// become checked loads (deka#1206).
#[allow(clippy::too_many_arguments)] // Independent module graph contexts belong to one lowering pass.
fn lower_module<'a>(
    path: &std::path::Path,
    ast: &deka_syntax::ast::Program<'a>,
    source: &str,
    hosts: &Hosts,
    host_exports: &deka_syntax::ModuleExports<'a>,
    arena: &'a bumpalo::Bump,
    module_exports: &HashMap<std::path::PathBuf, deka_syntax::ModuleExports<'a>>,
    struct_identities: &HashMap<std::path::PathBuf, BTreeMap<String, String>>,
    bindings: &HashMap<std::path::PathBuf, BTreeMap<String, usize>>,
    project: &Project,
    forward: Option<&BTreeMap<String, (String, std::path::PathBuf)>>,
    mark_checked: bool,
    // Harvest mode only: barrel entries whose target has not been harvested
    // yet are pushed here (exported name, original name, target) instead of
    // erroring, and resolved once every member is done.
    deferred: Option<&mut Vec<(String, String, std::path::PathBuf)>>,
    entry: &mut Context,
    lower: &mut Lower<'a>,
) -> Result<BTreeMap<String, usize>> {
    let mut deferred = deferred;
    lower.source_file = path.display().to_string();
    let mut imports = HashMap::new();
    for stmt in ast.statements {
        let source = match stmt {
            Stmt::Import { source, .. } => Some(*source),
            // The checker resolves re-exported types through the same map.
            Stmt::Export {
                decl:
                    ExportDecl::NamedGroup {
                        source: Some(source),
                        ..
                    },
                ..
            } => Some(*source),
            _ => None,
        };
        if let Some(source) = source {
            let exports = module_exports
                .get(&module_path(path, source, project)?)
                .ok_or("module was not loaded")?;
            imports.insert(source, exports);
        }
    }
    let globals = native_globals(hosts, host_exports, arena);
    let checked = deka_syntax::typeck::check_program_with_native_declarations(
        ast,
        source,
        &imports,
        &globals,
        Some(host_exports),
    );
    diagnostics(&checked.errors).map_err(|e| format!("{}: {e}", path.display()))?;
    entry.names.clear();
    entry.checked.clear();
    lower.import_json_factories(path, entry);
    lower.json_calls = checked
        .json_calls
        .iter()
        .map(|(p, call)| {
            (
                *p as usize,
                json_lower::Call {
                    operation: call.operation,
                    shape: lower.json_types.shape(&call.shape, path),
                    body_operation: call.body_operation.map(str::to_owned),
                },
            )
        })
        .collect();
    lower.optional_field_reads = checked
        .exception_forms
        .option_values
        .iter()
        .map(|expr| *expr as usize)
        .collect();
    lower.hosts.clear();
    lower.host_values.clear();
    lower.host_namespaces.clear();
    for op in hosts.namespaces() {
        let (namespace, field) = op.namespace.as_ref().expect("host namespace");
        lower
            .host_namespaces
            .entry(namespace.clone())
            .or_default()
            .push((field.clone(), op.name.clone()));
    }
    for op in hosts.globals() {
        if let Some(name) = &op.global_value {
            lower.host_values.insert(name.clone(), op.name.clone());
        } else {
            lower.hosts.insert(op.name.clone(), op.name.clone());
        }
    }
    for op in hosts.methods() {
        let (owner, method) = op.receiver_method.as_ref().expect("host method");
        lower.hosts.insert(
            deka_syntax::ast::mangle_method_name(method, owner),
            op.name.clone(),
        );
    }
    lower.declared.clear();
    lower.newtypes.clear();
    lower.structs.clear();
    lower.imported_structs.clear();
    lower.struct_identities = struct_identities[path].clone();
    lower.enums.clear();
    lower.enum_brands.clear();
    for (name, info) in &host_exports.enums {
        lower.enums.insert(
            (*name).into(),
            info.cases
                .iter()
                .map(|case| (case.name.into(), case.payload.is_some()))
                .collect(),
        );
        lower.enum_brands.insert((*name).into(), (*name).into());
    }
    for (name, info) in &host_exports.structs {
        lower.structs.insert(
            (*name).into(),
            info.fields
                .iter()
                .map(|f| (f.name.to_owned(), f.default_value.as_ref(), f.optional))
                .collect(),
        );
        lower
            .struct_identities
            .insert((*name).into(), (*name).into());
    }
    lower.method_decls.clear();
    lower.struct_embeds.clear();
    lower.exception_forms = checked
        .exception_forms
        .forms
        .iter()
        .map(|(p, form)| (*p as usize, *form))
        .collect();
    lower.exception_sources = checked
        .exception_forms
        .match_sources
        .iter()
        .map(|p| *p as usize)
        .collect();
    lower.catch_types = checked
        .exception_forms
        .catches
        .iter()
        .map(|(p, name)| (*p as usize, (*name).to_owned()))
        .collect();
    lower.enum_patterns = checked
        .enum_case_patterns
        .iter()
        .map(|(p, name)| (*p as usize, (*name).into()))
        .collect();
    lower.pattern_types = checked
        .union_type_patterns
        .iter()
        .map(|(p, test)| {
            use deka_syntax::typeck::UnionMemberTest::*;
            let predicate = match test {
                Primitive("void") => Ok(Op::MatchType(crate::TypeDescriptor::new("none", "None"))),
                Primitive(kind) => Ok(Op::MatchType(crate::TypeDescriptor::new(kind, kind))),
                Struct(name) => Ok(Op::MatchType(crate::TypeDescriptor::new("struct", name))),
                Enum(name) => Ok(Op::MatchType(crate::TypeDescriptor::new("enum", name))),
                EnumCase(name) => Ok(Op::MatchEnum {
                    name: Some((*name).into()),
                    case: String::new(),
                }),
                Bytes => Ok(Op::MatchType(crate::TypeDescriptor::new("bytes", "bytes"))),
                ErrorClass(_) => {
                    Err("this host type pattern is not supported by the native VM".into())
                }
            };
            (*p as usize, predicate)
        })
        .collect();
    lower.native_property_calls = checked
        .native_property_calls
        .iter()
        .map(|(site, op)| (*site as usize, (*op).to_owned()))
        .collect();
    lower.number_math_calls = checked
        .number_math_calls
        .keys()
        .map(|p| *p as usize)
        .collect();
    lower.type_of_calls = checked.type_of_calls.iter().map(|p| *p as usize).collect();
    lower.signature_calls = checked
        .signature_calls
        .iter()
        .map(|(p, tree)| (*p as usize, descriptor_summary(tree)))
        .collect();
    lower.newtype_results = checked
        .operator_rewrites
        .iter()
        .filter_map(|(p, op)| {
            use deka_syntax::typeck::OperatorRewrite::*;
            match op {
                NewtypeBinary { name } | NewtypeScalar { name, .. } | NewtypeUnary { name } => {
                    Some((*p as usize, (*name).to_string()))
                }
                _ => None,
            }
        })
        .collect();
    // Static method dispatch: the typechecker recorded each receiver-method
    // call site with the free function it rewrites to (and the embed path to
    // the declaring receiver). Keyed by expression address, like the
    // typechecker's own table.
    lower.method_calls = checked
        .method_calls
        .iter()
        .map(|(ptr, target)| {
            (
                *ptr as usize,
                (
                    target.mangled.clone(),
                    target.embed_path.iter().map(|s| (*s).to_string()).collect(),
                ),
            )
        })
        .collect();
    for stmt in ast.statements {
        if let Stmt::Newtype { name, .. } = stmt {
            lower.newtypes.insert((*name).into());
        }
        if let Stmt::Struct {
            name,
            fields,
            embeds,
            ..
        } = stmt
        {
            lower.structs.insert(
                (*name).into(),
                fields
                    .iter()
                    .map(|f| (f.name.to_string(), f.default_value.as_ref(), f.optional))
                    .collect(),
            );
            lower.struct_embeds.insert(
                (*name).into(),
                embeds.iter().map(|e| e.name.to_string()).collect(),
            );
        }
        let declared = match stmt {
            Stmt::Const { name, .. }
            | Stmt::Let { name, .. }
            | Stmt::UnwrapLet { name, .. }
            | Stmt::Function { name, .. } => Some(*name),
            Stmt::Export {
                decl: ExportDecl::Const { name, .. } | ExportDecl::Function { name, .. },
                ..
            } => Some(*name),
            _ => None,
        };
        if let Some(name) = declared {
            lower.declared.insert(name.into());
        }
        if let Stmt::ReceiverMethod {
            name,
            receiver_type,
            ..
        } = stmt
        {
            let mangled = deka_syntax::mangle_method_name(name, receiver_type);
            lower.declared.insert(mangled.clone());
            lower
                .method_decls
                .entry((*receiver_type).into())
                .or_default()
                .push(((*name).into(), mangled));
        }
        if let Stmt::Enum { name, cases, .. } = stmt {
            lower.enums.insert(
                (*name).into(),
                cases
                    .iter()
                    .map(|case| (case.name.to_string(), case.payload.is_some()))
                    .collect(),
            );
            for case in *cases {
                lower.declared.insert(format!("{}${}", name, case.name));
            }
        }
    }
    for stmt in ast.statements {
        if let Stmt::Import {
            source, specifiers, ..
        } = stmt
        {
            for spec in *specifiers {
                if spec.is_type_only && host_module(source) {
                    continue;
                }
                if host_module(source) {
                    if let Some(value) = builtins::constant(source, spec.imported) {
                        entry.emit(Op::Const(value));
                        let slot = entry.bind(spec.local);
                        entry.emit(Op::Store(slot));
                        continue;
                    }
                    let target = module_path(path, source, project)?;
                    if let Some(deka_syntax::ast::Type::Named { name: brand, .. }) =
                        module_exports[&target].aliases.get(spec.imported)
                    {
                        if let Some(info) = module_exports[&target].enums.get(spec.imported) {
                            lower.enums.insert(
                                spec.local.into(),
                                info.cases
                                    .iter()
                                    .map(|case| (case.name.into(), case.payload.is_some()))
                                    .collect(),
                            );
                            lower.enum_brands.insert(spec.local.into(), (*brand).into());
                            continue;
                        }
                        if let Some(info) = module_exports[&target].structs.get(spec.imported) {
                            lower.structs.insert(
                                spec.local.into(),
                                info.fields
                                    .iter()
                                    .map(|f| (f.name.into(), f.default_value.as_ref(), f.optional))
                                    .collect(),
                            );
                            lower
                                .struct_identities
                                .insert(spec.local.into(), (*brand).into());
                            continue;
                        }
                    }
                    let operation = builtins::operation(source, spec.imported, hosts)?;
                    lower.hosts.insert(spec.local.into(), operation);
                    continue;
                }
                let target = module_path(path, source, project)?;
                if let Some(info) = module_exports[&target].enums.get(spec.imported) {
                    lower.enums.insert(
                        spec.local.into(),
                        info.cases
                            .iter()
                            .map(|case| (case.name.into(), case.payload.is_some()))
                            .collect(),
                    );
                    let brand = match module_exports[&target].aliases.get(spec.imported) {
                        Some(deka_syntax::ast::Type::Named { name, .. }) => *name,
                        _ => spec.imported,
                    };
                    lower.enum_brands.insert(spec.local.into(), brand.into());
                    continue;
                }
                if let Some(info) = module_exports[&target].structs.get(spec.imported) {
                    lower.imported_structs.insert(spec.local.to_owned());
                    lower.struct_identities.insert(
                        spec.local.into(),
                        struct_identities[&target][spec.imported].clone(),
                    );
                    lower.structs.insert(
                        spec.local.into(),
                        info.fields
                            .iter()
                            .map(|f| (f.name.to_owned(), f.default_value.as_ref(), f.optional))
                            .collect(),
                    );
                    lower.struct_embeds.insert(
                        spec.local.into(),
                        info.embeds.iter().map(|e| e.name.to_owned()).collect(),
                    );
                    // Struct names are type bindings, not runtime cells.
                    continue;
                }
                if spec.is_type_only {
                    // Type-only imports resolve for the checker (which rejects
                    // value uses) and erase here: no slot, no host lookup.
                    continue;
                }
                if let Some((imported, origin)) =
                    forward.and_then(|f| f.get(spec.local).map(|f| (&f.0, &f.1)))
                {
                    debug_assert_eq!(imported.as_str(), spec.imported);
                    // Reserve a local in both passes so the harvest and real
                    // allocation sequences stay identical.
                    entry.bind(spec.local);
                    if mark_checked {
                        // The slot comes from the module the import names
                        // (a barrel re-exports it); the error names the
                        // origin whose assignment fills it.
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
                                origin.display(),
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
    lower.reserve_json_factories(path, entry);
    // Receiver methods hoist: a method call may precede the declaration
    // (methods are type-level; values are not), so every method lowers before
    // any other statement. Enum declarations hoist with them: their interned
    // payload-free cases bind before any use. A body referencing a module
    // value declared later is a forward-reference error.
    for stmt in ast.statements {
        if matches!(stmt, Stmt::ReceiverMethod { .. } | Stmt::Enum { .. }) {
            lower.statement(stmt, entry)?;
        }
    }
    lower.finish_json_factories(entry)?;
    for stmt in ast.statements {
        if matches!(stmt, Stmt::ReceiverMethod { .. } | Stmt::Enum { .. }) {
            continue;
        }
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
                ExportDecl::NamedGroup { names, source } => match source {
                    // `export { x } from "./y.ds"`: the barrel aliases the
                    // target's slot — no copy, so timing behaves as if the
                    // consumer imported from the origin directly (deka#1210).
                    Some(source) if host_module(source) => {
                        let target = module_path(path, source, project)?;
                        for name in *names {
                            if module_exports[&target].structs.contains_key(name.name)
                                || module_exports[&target].enums.contains_key(name.name)
                            {
                                continue;
                            }
                            if let Some(value) = builtins::constant(source, name.name) {
                                entry.emit(Op::Const(value));
                            } else {
                                let operation = builtins::operation(source, name.name, hosts)?;
                                lower.host_closure(&operation, entry)?;
                            }
                            let external = name.alias.unwrap_or(name.name);
                            let slot = entry.bind(external);
                            entry.emit(Op::Store(slot));
                            exported.insert(external.into(), slot);
                        }
                    }
                    Some(source) => {
                        let target = module_path(path, source, project)?;
                        for name in *names {
                            if erased_export(&module_exports[&target], name.name) {
                                continue;
                            }
                            let external = name.alias.unwrap_or(name.name);
                            let slot = bindings.get(&target).and_then(|b| b.get(name.name));
                            match (slot, deferred.as_deref_mut()) {
                                (Some(slot), _) => {
                                    exported.insert(external.to_string(), *slot);
                                }
                                (None, Some(pending)) => {
                                    pending.push((
                                        external.to_string(),
                                        name.name.to_string(),
                                        target.clone(),
                                    ));
                                }
                                (None, None) => {
                                    return Err(format!(
                                        "{}: {}:{}: module {} does not export `{}`",
                                        path.display(),
                                        name.span.start.line,
                                        name.span.start.column,
                                        target.display(),
                                        name.name
                                    ));
                                }
                            }
                        }
                    }
                    // `export { a, b as c }`: re-export local values;
                    // type-only names erase (the checker tracks them).
                    None => {
                        for name in *names {
                            if let Ok(slot) = entry.slot(name.name) {
                                exported.insert(name.alias.unwrap_or(name.name).into(), slot);
                            }
                        }
                    }
                },
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
    project: &Project,
    package_exports: Option<&mut Vec<PackageExport>>,
) -> Result<Program> {
    let arena = bumpalo::Bump::new();
    let declarations = hosts.declarations()
        + &PromiseJoin::ALL
            .into_iter()
            .map(PromiseJoin::declaration)
            .collect::<String>();
    let host_parse = deka_syntax::parse(&declarations, &arena);
    diagnostics(&host_parse.errors)?;
    let host_ast = host_parse.program.ok_or("missing host declarations")?;
    let mut host_exports = deka_syntax::collect_module_exports(&host_ast, &arena);
    for op in hosts.properties() {
        let (_, property) = op.receiver_method.as_ref().expect("host property");
        let (operation, ty) = host_exports
            .values
            .get_key_value(op.name.as_str())
            .expect("host declaration");
        let deka_syntax::typeck::Type::Function { params, ret, .. } = ty else {
            return Err("host property declaration is not callable".into());
        };
        let deka_syntax::typeck::Type::Opaque { identity, .. } = params[0] else {
            return Err("host property needs an opaque receiver".into());
        };
        host_exports.native_properties.insert(
            (identity, arena.alloc_str(property)),
            (*operation, *ret.clone()),
        );
    }
    for op in hosts.methods().filter(|op| op.json_body) {
        let (_, method) = op.receiver_method.as_ref().expect("host method");
        let (operation, ty) = host_exports
            .values
            .get_key_value(op.name.as_str())
            .expect("host declaration");
        let deka_syntax::typeck::Type::Function { params, .. } = ty else {
            return Err("JSON body declaration is not callable".into());
        };
        let deka_syntax::typeck::Type::Opaque { identity, .. } = params[0] else {
            return Err("JSON body needs an opaque receiver".into());
        };
        host_exports
            .native_json_bodies
            .insert((identity, arena.alloc_str(method)), *operation);
    }
    // Parse every module up front so import checks resolve across a cycle.
    let mut asts = HashMap::new();
    let mut module_exports = builtins::exports(hosts, &host_exports, &arena);
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
    // The module graph: importer → (local, exported name, target) for value
    // imports, and barrel → (exported name, original name, target) for
    // `export { x } from` re-exports.
    let mut edges: HashMap<std::path::PathBuf, Vec<(String, String, std::path::PathBuf)>> =
        HashMap::new();
    let mut barrels: HashMap<std::path::PathBuf, Vec<(String, String, std::path::PathBuf)>> =
        HashMap::new();
    for (path, _source) in modules {
        for stmt in asts[path].statements {
            match stmt {
                Stmt::Import {
                    source, specifiers, ..
                } if !host_module(source) => {
                    let target = module_path(path, source, project)?;
                    for spec in *specifiers {
                        edges.entry(path.clone()).or_default().push((
                            spec.local.into(),
                            spec.imported.into(),
                            target.clone(),
                        ));
                    }
                }
                Stmt::Export {
                    decl:
                        ExportDecl::NamedGroup {
                            names,
                            source: Some(source),
                        },
                    ..
                } => {
                    let target = module_path(path, source, project)?;
                    for name in *names {
                        barrels.entry(path.clone()).or_default().push((
                            name.alias.unwrap_or(name.name).into(),
                            name.name.into(),
                            target.clone(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    // Flatten re-export chains so the checker resolves names through
    // barrels without deka_syntax changes: copy the origin's export entries
    // under the barrel's external names. Barrel cycles (re-export-only
    // loops) are skipped here and load through the runtime machinery.
    fn augment_reexports(
        module: &std::path::Path,
        barrels: &HashMap<std::path::PathBuf, Vec<(String, String, std::path::PathBuf)>>,
        module_exports: &mut HashMap<std::path::PathBuf, deka_syntax::ModuleExports<'_>>,
        visiting: &mut Vec<std::path::PathBuf>,
    ) {
        if visiting.contains(&module.to_path_buf()) {
            return;
        }
        visiting.push(module.to_path_buf());
        let Some(entries) = barrels.get(module).cloned() else {
            visiting.pop();
            return;
        };
        for (external, original, target) in entries {
            augment_reexports(&target, barrels, module_exports, visiting);
            let Some(source) = module_exports.get(&target).cloned() else {
                continue;
            };
            let Some(exports) = module_exports.get_mut(module) else {
                continue;
            };
            if let Some(ty) = source.values.get(original.as_str()) {
                exports
                    .values
                    .insert(Box::leak(external.clone().into_boxed_str()), ty.clone());
            }
            if let Some(info) = source.structs.get(original.as_str()) {
                exports
                    .structs
                    .insert(Box::leak(external.clone().into_boxed_str()), info.clone());
            }
            if let Some(info) = source.enums.get(original.as_str()) {
                exports
                    .enums
                    .insert(Box::leak(external.clone().into_boxed_str()), info.clone());
            }
            if let Some(ty) = source.aliases.get(original.as_str()) {
                exports
                    .aliases
                    .insert(Box::leak(external.clone().into_boxed_str()), ty.clone());
            }
            if let Some(ty) = source.opaques.get(original.as_str()) {
                exports
                    .opaques
                    .insert(Box::leak(external.clone().into_boxed_str()), ty.clone());
            }
            if let Some(info) = source.newtypes.get(original.as_str()) {
                exports
                    .newtypes
                    .insert(Box::leak(external.clone().into_boxed_str()), info.clone());
            }
            if let Some(info) = source.interfaces.get(original.as_str()) {
                exports
                    .interfaces
                    .insert(Box::leak(external.clone().into_boxed_str()), info.clone());
            }
            if let Some(tree) = source.build_fragments.get(original.as_str()) {
                exports
                    .build_fragments
                    .insert(Box::leak(external.clone().into_boxed_str()), tree.clone());
            }
            if source.interactive_components.contains(original.as_str()) {
                exports
                    .interactive_components
                    .insert(Box::leak(external.clone().into_boxed_str()));
            }
            if external == "default" {
                exports.default_export_declared_name = Some(Box::leak(original.into_boxed_str()));
            }
        }
        visiting.pop();
    }
    for (path, _source) in modules {
        augment_reexports(path, &barrels, &mut module_exports, &mut Vec::new());
    }

    let globals = native_globals(hosts, &host_exports, &arena);
    for _ in 0..=modules.len() {
        let before: Vec<_> = modules
            .iter()
            .map(|(path, _)| module_exports[path].values.clone())
            .collect();
        for (path, _) in modules {
            let mut exports = module_exports[path].clone();
            let mut imports = HashMap::new();
            for stmt in asts[path].statements {
                if let Stmt::Import { source, .. } = stmt {
                    let imported = &module_exports[&module_path(path, source, project)?];
                    imports.insert(*source, imported);
                }
            }
            deka_syntax::typeck::refresh_module_export_values_with_native_declarations(
                asts[path],
                &imports,
                &mut exports,
                &globals,
                Some(&host_exports),
            );
            module_exports.insert(path.clone(), exports);
        }
        for (path, _) in modules {
            augment_reexports(path, &barrels, &mut module_exports, &mut Vec::new());
        }
        if modules
            .iter()
            .zip(before)
            .all(|((path, _), values)| module_exports[path].values == values)
        {
            break;
        }
    }

    // JSON descriptors are built in their declaring namespace. Export only
    // the roots: nested private factory schemas remain compiler metadata.
    for (path, _) in modules {
        let mut imports = HashMap::new();
        for stmt in asts[path].statements {
            if let Stmt::Import { source, .. } = stmt {
                let exports = &module_exports[&module_path(path, source, project)?];
                imports.insert(*source, exports);
            }
        }
        let fragments = deka_syntax::build_module_build_fragments(asts[path], &imports);
        let exports = module_exports.get_mut(path).expect("parsed module exports");
        for stmt in asts[path].statements {
            if let Stmt::Export {
                decl:
                    ExportDecl::NamedGroup {
                        names,
                        source: None,
                    },
                ..
            } = stmt
            {
                for name in *names {
                    if let Some(tree) = fragments.get(name.name) {
                        exports
                            .build_fragments
                            .insert(name.alias.unwrap_or(name.name), tree.clone());
                    }
                }
            }
        }
        for (module, _) in modules {
            augment_reexports(module, &barrels, &mut module_exports, &mut Vec::new());
        }
    }

    // Nominal struct identity follows declarations through aliases and barrels.
    // Module ordinals keep filesystem paths out of serialized applications.
    let mut struct_identities: HashMap<std::path::PathBuf, BTreeMap<String, String>> =
        module_exports
            .keys()
            .map(|path| (path.clone(), BTreeMap::new()))
            .collect();
    for (path, exports) in &module_exports {
        if builtins::is_path(path) {
            for (name, alias) in &exports.aliases {
                if exports.structs.contains_key(name)
                    && let deka_syntax::ast::Type::Named { name: brand, .. } = alias
                {
                    struct_identities
                        .get_mut(path)
                        .expect("builtin identity map")
                        .insert((*name).into(), (*brand).into());
                }
            }
        }
    }
    for (i, (path, _)) in modules.iter().enumerate() {
        let mut names = BTreeMap::new();
        for stmt in asts[path].statements {
            if let Stmt::Struct { name, .. } = stmt {
                names.insert((*name).to_owned(), format!("struct:{i}:{name}"));
            }
        }
        struct_identities.insert(path.clone(), names);
    }
    loop {
        let mut changed = false;
        for (path, _) in modules {
            for stmt in asts[path].statements {
                let Stmt::Export {
                    decl: ExportDecl::NamedGroup { names, source },
                    ..
                } = stmt
                else {
                    continue;
                };
                let target = match source {
                    Some(source) => module_path(path, source, project)?,
                    None => path.clone(),
                };
                for name in *names {
                    let Some(identity) = struct_identities[&target].get(name.name).cloned() else {
                        continue;
                    };
                    let external = name.alias.unwrap_or(name.name).to_owned();
                    if !struct_identities[path].contains_key(&external) {
                        struct_identities
                            .get_mut(path)
                            .unwrap()
                            .insert(external, identity);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // A module is a cycle member when it can reach itself through the graph
    // (following value imports and barrel re-exports alike).
    let mut members: std::collections::BTreeSet<std::path::PathBuf> = Default::default();
    for (path, _source) in modules {
        let mut seen: std::collections::BTreeSet<std::path::PathBuf> = Default::default();
        let mut queue = vec![path.clone()];
        while let Some(module) = queue.pop() {
            let targets = edges
                .get(&module)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|(_, _, target)| target)
                .chain(
                    barrels
                        .get(&module)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(_, _, target)| target),
                );
            for target in targets {
                if target == *path {
                    members.insert(path.clone());
                }
                if seen.insert(target.clone()) {
                    queue.push(target);
                }
            }
        }
    }
    // The module that actually initializes an exported name: follows barrel
    // chains to the origin, so checked reads and sentinel marking key on the
    // module whose assignment fills the slot. A barrel cycle resolves to its
    // latest-initializing member, which only ever makes reads *more* checked.
    let resolve_origin = |module: &std::path::Path,
                          name: &str,
                          position: &HashMap<std::path::PathBuf, usize>|
     -> std::path::PathBuf {
        let mut current = (module.to_path_buf(), name.to_string());
        let mut visited = vec![current.0.clone()];
        loop {
            let hop = barrels
                .get(&current.0)
                .and_then(|entries| {
                    entries
                        .iter()
                        .find(|(exported, _, _)| *exported == current.1)
                })
                .map(|(_, original, target)| (target.clone(), original.clone()));
            let Some(next) = hop else {
                return current.0;
            };
            if visited.contains(&next.0) {
                return visited
                    .into_iter()
                    .max_by_key(|module| position[module])
                    .unwrap_or(current.0);
            }
            // Builtin functions are installed in the barrel's own initialization.
            if builtins::is_path(&next.0) {
                return current.0;
            }
            visited.push(next.0.clone());
            current = next;
        }
    };
    // An import whose origin initializes after the importer (later in loaded
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
            let origin = resolve_origin(target, imported, &position);
            if position[&origin] > position[module] {
                forward
                    .entry(module.clone())
                    .or_default()
                    .insert(local.clone(), (imported.clone(), origin));
            }
        }
    }
    let mut bindings: HashMap<std::path::PathBuf, BTreeMap<String, usize>> = HashMap::new();
    let mut lower = Lower {
        source_file: String::new(),
        functions: vec![],
        hosts: BTreeMap::new(),
        host_namespaces: BTreeMap::new(),
        host_values: BTreeMap::new(),
        optional_field_reads: Default::default(),
        host_arities: hosts.declarations_names_and_arities(),
        declared: std::collections::BTreeSet::new(),
        newtypes: std::collections::BTreeSet::new(),
        structs: BTreeMap::new(),
        struct_identities: BTreeMap::new(),
        imported_structs: Default::default(),
        method_calls: HashMap::new(),
        enums: BTreeMap::new(),
        enum_brands: BTreeMap::new(),
        method_decls: BTreeMap::new(),
        struct_embeds: BTreeMap::new(),
        native_property_calls: Default::default(),
        number_math_calls: Default::default(),
        type_of_calls: Default::default(),
        exception_forms: HashMap::new(),
        exception_sources: Default::default(),
        catch_types: HashMap::new(),
        enum_patterns: HashMap::new(),
        pattern_types: HashMap::new(),
        signature_calls: HashMap::new(),
        json_calls: HashMap::new(),
        json_types: json_lower::JsonTypes::new(&asts, &struct_identities, &edges),
        json_factories: Default::default(),
        newtype_results: HashMap::new(),
        console_outputs: ["echo", CONSOLE_ERROR_OPERATION]
            .into_iter()
            .filter(|name| {
                hosts.operation(name).is_ok_and(|op| {
                    op.args == [crate::HostType::String]
                        && op.result == crate::HostType::Unit
                        && !op.asynchronous
                        && !op.result_channel
                })
            })
            .collect(),
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
            // Members harvested earlier in this pass resolve for later ones.
            let mut scratch_bindings = bindings.clone();
            let mut deferred_all: HashMap<
                std::path::PathBuf,
                Vec<(String, String, std::path::PathBuf)>,
            > = HashMap::new();
            for (member, member_source) in modules {
                if !members.contains(member) {
                    continue;
                }
                let mut deferred = vec![];
                let exported = lower_module(
                    member,
                    asts[member],
                    member_source,
                    hosts,
                    &host_exports,
                    &arena,
                    &module_exports,
                    &struct_identities,
                    &scratch_bindings,
                    project,
                    forward.get(member),
                    false,
                    Some(&mut deferred),
                    &mut entry,
                    &mut lower,
                )?;
                deferred_all.insert(member.clone(), deferred);
                harvested.insert(member.clone(), exported.clone());
                scratch_bindings.insert(member.clone(), exported);
            }
            // Second harvest pass: barrel entries whose target was still
            // pending now resolve against the completed member exports.
            for (member, pending) in &deferred_all {
                for (external, original, target) in pending {
                    let slot = *scratch_bindings
                        .get(target)
                        .and_then(|b| b.get(original))
                        .ok_or("missing module export")?;
                    harvested
                        .get_mut(member)
                        .expect("member harvested")
                        .insert(external.clone(), slot);
                    scratch_bindings
                        .get_mut(member)
                        .expect("member harvested")
                        .insert(external.clone(), slot);
                }
            }
            entry = snapshot;
            lower.functions.truncate(functions_len);
            // Sentinels cover only slots whose origin is a cycle member: a
            // member barrel re-exporting an already-initialized non-member
            // must not be clobbered.
            let mut sentinel_slots: std::collections::BTreeSet<usize> = Default::default();
            for (member, exported) in &harvested {
                for (name, slot) in exported {
                    if members.contains(&resolve_origin(member, name, &position)) {
                        sentinel_slots.insert(*slot);
                    }
                }
            }
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
            &arena,
            &module_exports,
            &struct_identities,
            &bindings,
            project,
            forward.get(path),
            true,
            None,
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
    if let Some(output) = package_exports {
        let (path, _) = modules.last().ok_or("missing package entry")?;
        let surface = &module_exports[path];
        let names = surface
            .values
            .keys()
            .chain(surface.interfaces.keys())
            .chain(surface.structs.keys())
            .chain(surface.enums.keys())
            .chain(surface.aliases.keys())
            .chain(surface.opaques.keys())
            .chain(surface.newtypes.keys())
            .chain(surface.re_exports.iter())
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        output.extend(names.into_iter().map(|name| PackageExport {
            name: name.to_owned(),
            type_only: erased_export(surface, name),
        }));
    }
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
/// Patch lists for one enclosing loop: jumps emitted by `break` and
/// `continue` inside its body, resolved when the loop is fully lowered.
#[derive(Clone, Default)]
struct LoopTargets {
    breaks: Vec<usize>,
    continues: Vec<usize>,
    handler_depth: usize,
}
#[derive(Clone)]
struct Context {
    function: Function,
    names: BTreeMap<String, usize>,
    /// Names whose reads must check for the `Uninitialized` sentinel at
    /// runtime: imports across a cycle-closing edge, mapped to the error
    /// naming the export and both files (deka#1206).
    checked: BTreeMap<String, String>,
    loops: Vec<LoopTargets>,
    handler_depth: usize,
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
            loops: vec![],
            handler_depth: 0,
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
        self.patch_to(at, end);
    }
    fn patch_to(&mut self, at: usize, target: usize) {
        match &mut self.function.code[at] {
            Op::Jump(i) | Op::JumpIfFalse(i) | Op::JumpIfUnit(i) | Op::Handler(i) => *i = target,
            _ => unreachable!(),
        }
    }
    /// Resolve a loop's `continue` jumps to its step and its `break` jumps to
    /// just after it, then pop it off the loop stack.
    fn finish_loop(&mut self, continue_target: usize) {
        let targets = self.loops.pop().unwrap();
        for at in &targets.continues {
            self.patch_to(*at, continue_target);
        }
        let end = self.function.code.len();
        for at in &targets.breaks {
            self.patch_to(*at, end);
        }
    }
}
/// What a lowered closure runs: a statement body (falling off the end
/// returns unit) or a single returned expression.
enum ClosureBody<'s, 'a> {
    Block(&'s [Stmt<'a>]),
    Value(&'s Expr<'a>),
}
/// A struct declaration's fields in declaration order: name, default
/// expression, and whether the field is optional.
type StructFields<'a> = Vec<(String, Option<&'a Expr<'a>>, bool)>;
struct Lower<'a> {
    source_file: String,
    functions: Vec<Function>,
    hosts: BTreeMap<String, String>,
    host_namespaces: BTreeMap<String, Vec<(String, String)>>,
    host_values: BTreeMap<String, String>,
    optional_field_reads: std::collections::HashSet<usize>,
    host_arities: BTreeMap<String, usize>,
    /// Top-level names declared anywhere in the current module. A call to one
    /// of these before its declaration is a forward reference; a call to any
    /// other unbound name is an unknown built-in.
    declared: std::collections::BTreeSet<String>,
    /// Newtype names declared in the current module. A call to one is a
    /// constructor; the primitive payload keeps nominal heap metadata for
    /// reflection, while arithmetic follows the checker's result type.
    newtypes: std::collections::BTreeSet<String>,
    /// Struct declarations in the current module. A struct value is a record;
    /// the declaration only matters when a literal omits fields.
    structs: BTreeMap<String, StructFields<'a>>,
    struct_identities: BTreeMap<String, String>,
    imported_structs: std::collections::BTreeSet<String>,
    /// Receiver-method call sites in the current module, keyed by expression
    /// address: the free function the call rewrites to and the embed path to
    /// the declaring receiver. Recorded by the typechecker.
    method_calls: HashMap<usize, (String, Vec<String>)>,
    /// Enum declarations in the current module: case names in declaration
    /// order with which carry a payload. A variant is a record
    /// `{name, index[, value]}`; payload-free cases are interned one record
    /// per declaration, hoisted with the receiver methods.
    enums: BTreeMap<String, Vec<(String, bool)>>,
    enum_brands: BTreeMap<String, String>,
    /// Methods declared in the current module per receiver type: method key
    /// and mangled free-function name. Struct literals attach them to the
    /// record under `$<key>` so an interface-typed call finds them at run
    /// time.
    method_decls: BTreeMap<String, Vec<(String, String)>>,
    /// Embedded struct names per struct declaration, for promoted-method
    /// attachment.
    struct_embeds: BTreeMap<String, Vec<String>>,
    /// The host registry supplies the output sink and its wire signature.
    console_outputs: std::collections::BTreeSet<&'static str>,
    native_property_calls: HashMap<usize, String>,
    number_math_calls: std::collections::BTreeSet<usize>,
    type_of_calls: std::collections::BTreeSet<usize>,
    exception_forms: HashMap<usize, ExceptionEmit>,
    exception_sources: std::collections::BTreeSet<usize>,
    catch_types: HashMap<usize, String>,
    enum_patterns: HashMap<usize, String>,
    pattern_types: HashMap<usize, Result<Op>>,
    signature_calls: HashMap<usize, crate::TypeDescriptor>,
    json_calls: HashMap<usize, json_lower::Call>,
    json_types: json_lower::JsonTypes,
    json_factories: BTreeMap<String, (std::path::PathBuf, usize)>,
    newtype_results: HashMap<usize, String>,
}
impl<'a> Lower<'a> {
    /// Recursive lowering keeps the innermost refusal and its module once.
    fn refusal(&self, span: Span, error: String) -> String {
        source_refusal(&self.source_file, span, error)
    }
    /// Prelude cases have the same ordered schema and bytecode as declared enums.
    fn enum_cases(&self, name: &str) -> Option<Vec<(String, bool)>> {
        self.enums.get(name).cloned().or_else(|| {
            let cases: &[(&str, bool)] = match name {
                "Option" => &[("Some", true), ("None", false)],
                "Result" => &[("Ok", true), ("Err", true)],
                _ => return None,
            };
            Some(cases.iter().map(|(n, p)| ((*n).into(), *p)).collect())
        })
    }
    fn enum_constructor(
        &mut self,
        name: &str,
        case: &str,
        payload: Option<&Expr<'a>>,
        c: &mut Context,
    ) -> Result<()> {
        let cases = self
            .enum_cases(name)
            .ok_or_else(|| format!("enum `{name}` is not declared in this module"))?;
        let (index, (_, has_payload)) = cases
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n == case)
            .ok_or_else(|| format!("case `{case}` not found in enum `{name}`"))?;
        if *has_payload != payload.is_some() {
            return Err(format!(
                "case `{case}` of enum `{name}` has the wrong payload arity"
            ));
        }
        if let Some(value) = payload {
            self.expr(value, c)?;
        } else if self.enums.contains_key(name) && !self.enum_brands.contains_key(name) {
            // Payload-free declared cases remain interned at their declaration.
            c.emit_load(&format!("{name}${case}"))?;
            return Ok(());
        }
        c.emit(Op::Enum {
            name: self
                .enum_brands
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.into()),
            case: case.into(),
            index,
            payload: *has_payload,
        });
        Ok(())
    }
    /// Every method visible on a value of `type_name`: its own plus those
    /// promoted through embedded structs, as (method key, mangled name).
    fn methods_for(&self, type_name: &str, seen: &mut Vec<String>) -> Vec<(String, String)> {
        if seen.iter().any(|s| s == type_name) {
            return vec![];
        }
        seen.push(type_name.into());
        let mut out = self
            .method_decls
            .get(type_name)
            .cloned()
            .unwrap_or_default();
        if let Some(embeds) = self.struct_embeds.get(type_name).cloned() {
            for embed in embeds {
                out.extend(self.methods_for(&embed, seen));
            }
        }
        out
    }
    /// Attach a struct value's methods to the record on the stack under
    /// `$<key>`, so an interface-typed `MethodCall` finds them at run time.
    fn attach_methods(&self, type_name: &str, c: &mut Context) -> Result<()> {
        for (key, mangled) in self.methods_for(type_name, &mut vec![]) {
            c.emit_load(&mangled)?;
            c.emit(Op::Record(vec![format!("${key}")]));
            c.emit(Op::RecordExtend);
        }
        Ok(())
    }
    fn struct_value(
        &mut self,
        name: &str,
        supplied: &[(String, usize)],
        c: &mut Context,
    ) -> Result<()> {
        let declared = self.structs.get(name).cloned().unwrap_or_default();
        let embeds = self.struct_embeds.get(name).cloned().unwrap_or_default();
        let mut names = vec![];
        // Preserve written order for the fields owned by this struct.
        for (field, slot) in supplied {
            if declared.iter().any(|(n, _, _)| n == field) {
                c.emit(Op::Load(*slot));
                names.push(field.clone());
            }
        }
        for (field, default, optional) in declared {
            if optional || names.contains(&field) {
                continue;
            }
            if let Some(default) = default {
                if self.imported_structs.contains(name) {
                    return Err(format!(
                        "imported struct `{name}` default `{field}` requires declaration-module evaluation"
                    ));
                }
                self.expr(default, c)?;
                names.push(field);
            }
        }
        for embed in &embeds {
            if let Some((_, slot)) = supplied.iter().find(|(n, _)| n == embed) {
                c.emit(Op::Load(*slot));
            } else {
                self.struct_value(embed, supplied, c)?;
            }
            names.push(embed.clone());
        }
        c.emit(Op::Struct {
            name: name.into(),
            identity: self.struct_identities.get(name).cloned(),
            fields: names,
            embeds,
        });
        self.attach_methods(name, c)
    }
    fn scoped(&mut self, body: &[Stmt<'a>], c: &mut Context) -> Result<()> {
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
        params: &[Param<'a>],
        body: &[Stmt<'a>],
        asynchronous: bool,
        outer: &mut Context,
    ) -> Result<()> {
        self.closure(name, params, ClosureBody::Block(body), asynchronous, outer)
    }
    /// A synchronous zero-parameter closure returning `value`: markup
    /// bindings, component prop getters and dynamic UI attributes. The
    /// expression is lowered in place, never copied, because the typechecker
    /// keys receiver-method call sites by expression address.
    fn thunk(&mut self, name: &str, value: &Expr<'a>, outer: &mut Context) -> Result<()> {
        self.closure(name, &[], ClosureBody::Value(value), false, outer)
    }
    fn closure(
        &mut self,
        name: &str,
        params: &[Param<'a>],
        body: ClosureBody<'_, 'a>,
        asynchronous: bool,
        outer: &mut Context,
    ) -> Result<()> {
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
        let mut tuple_params = vec![];
        let mut defaults = vec![];
        for p in params {
            let slot = match &p.binding {
                ParamBinding::Identifier(name) => c.bind(name),
                ParamBinding::Tuple(names) => {
                    let slot = c.bind(&format!("<tuple param {}>", c.function.locals));
                    tuple_params.push((slot, *names));
                    slot
                }
            };
            if let Some(default) = &p.default_value {
                defaults.push((slot, default));
            }
        }
        // Defaults run first: an omitted argument arrives as unit and the
        // prologue fills it before any destructuring reads the slot.
        for (slot, default) in defaults {
            c.emit(Op::Load(slot));
            let fill = c.emit(Op::JumpIfUnit(0));
            let done = c.emit(Op::Jump(0));
            c.patch(fill);
            self.expr(default, &mut c)?;
            c.emit(Op::Store(slot));
            c.patch(done);
        }
        for (slot, names) in tuple_params {
            for (i, name) in names.iter().enumerate() {
                c.emit(Op::Load(slot));
                c.emit(Op::Const(Literal::Number(i as f64)));
                c.emit(Op::Index);
                let local = c.bind(name);
                c.emit(Op::Store(local));
            }
        }
        let index = self.functions.len();
        self.functions.push(c.function.clone());
        match body {
            ClosureBody::Block(body) => {
                for s in body {
                    self.statement(s, &mut c)?;
                }
                c.emit(Op::Const(Literal::Unit));
            }
            ClosureBody::Value(value) => self.expr(value, &mut c)?,
        }
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
        params: &[Param<'a>],
        _types: &[TypeParam<'a>],
        body: &[Stmt<'a>],
        asynchronous: bool,
        c: &mut Context,
    ) -> Result<()> {
        // Type parameters erase: one copy of the code serves every
        // instantiation.
        let slot = c.bind(name);
        self.function(name, params, body, asynchronous, c)?;
        c.emit(Op::Store(slot));
        Ok(())
    }
    /// APS 30 pipe: `a |> f` calls `f(a)`, `a |> f(b)` calls `f(a, b)`, and a
    /// `_` in the argument list marks the slot the left side fills.
    fn pipe(&mut self, left: &Expr<'a>, right: &Expr<'a>, c: &mut Context) -> Result<()> {
        let value = c.bind(&format!("<pipe value {}>", c.function.locals));
        self.expr(left, c)?;
        c.emit(Op::Store(value));
        if let Expr::Call { callee, args, .. } = right {
            // Explicit type arguments erase.
            let has_hole = args
                .iter()
                .any(|a| matches!(a, Expr::Identifier { name: "_", .. }));
            self.expr(callee, c)?;
            let mut argc = args.len();
            if !has_hole {
                c.emit(Op::Load(value));
                argc += 1;
            }
            for arg in *args {
                if matches!(arg, Expr::Identifier { name: "_", .. }) {
                    c.emit(Op::Load(value));
                } else {
                    self.expr(arg, c)?;
                }
            }
            c.emit(Op::Call(argc));
        } else {
            self.expr(right, c)?;
            c.emit(Op::Load(value));
            c.emit(Op::Call(1));
        }
        Ok(())
    }
    fn pattern_bind(&self, name: &str, value: usize, c: &mut Context) {
        c.emit(Op::Load(value));
        let slot = c.bind(name);
        if !c.loops.is_empty() {
            c.emit(Op::Rebind(slot));
        }
        c.emit(Op::Store(slot));
    }
    /// Every refutable step jumps to the next arm. A payload is only read
    /// after its outer constructor matches; failed nested tests leave no
    /// operand-stack residue and the arm's names cannot escape its scope.
    fn pattern(
        &mut self,
        p: &Pattern<'a>,
        value: usize,
        c: &mut Context,
        failed: &mut Vec<usize>,
    ) -> Result<()> {
        match p {
            Pattern::Wildcard { .. } => {}
            Pattern::Identifier { name, .. } => {
                if let Some(case) = self.enum_patterns.get(&(p as *const Pattern as usize)) {
                    c.emit(Op::Load(value));
                    c.emit(Op::MatchEnum {
                        name: None,
                        case: case.clone(),
                    });
                    failed.push(c.emit(Op::JumpIfFalse(0)));
                } else {
                    self.pattern_bind(name, value, c);
                }
            }
            Pattern::Literal { expr, .. } => {
                c.emit(Op::Load(value));
                self.expr(expr, c)?;
                c.emit(Op::MatchEqual);
                failed.push(c.emit(Op::JumpIfFalse(0)));
            }
            Pattern::Constructor { name, payload, .. } => {
                let mut predicate = match self.pattern_types.get(&(p as *const Pattern as usize)) {
                    Some(predicate) => predicate.clone()?,
                    None => Op::MatchEnum {
                        name: None,
                        case: (*name).into(),
                    },
                };
                if let Op::MatchEnum { case, .. } = &mut predicate {
                    *case = (*name).into();
                }
                if let Op::MatchType(crate::TypeDescriptor { kind, name }) = &predicate
                    && kind == "struct"
                    && let Some(identity) = self.struct_identities.get(name)
                {
                    predicate = Op::MatchStruct(identity.clone());
                }
                let extracts_payload = matches!(predicate, Op::MatchEnum { .. });
                c.emit(Op::Load(value));
                c.emit(predicate);
                failed.push(c.emit(Op::JumpIfFalse(0)));
                if let Some(p) = payload {
                    let inner = if extracts_payload {
                        c.emit(Op::Load(value));
                        c.emit(Op::Field("value".into()));
                        let inner = c.bind(&format!("<pattern payload {}>", c.function.locals));
                        c.emit(Op::Store(inner));
                        inner
                    } else {
                        value
                    };
                    self.pattern(p, inner, c, failed)?;
                }
            }
            Pattern::Or { alternatives, .. } => {
                let mut matched = vec![];
                for (i, alternative) in alternatives.iter().enumerate() {
                    let mut next = vec![];
                    self.pattern(alternative, value, c, &mut next)?;
                    if i + 1 == alternatives.len() {
                        failed.extend(next);
                    } else {
                        matched.push(c.emit(Op::Jump(0)));
                        for jump in next {
                            c.patch(jump);
                        }
                    }
                }
                for jump in matched {
                    c.patch(jump);
                }
            }
            Pattern::Tuple { elements, .. } => {
                c.emit(Op::Load(value));
                c.emit(Op::MatchTuple(elements.len()));
                failed.push(c.emit(Op::JumpIfFalse(0)));
                for (i, p) in elements.iter().enumerate() {
                    c.emit(Op::Load(value));
                    c.emit(Op::Const(Literal::Number(i as f64)));
                    c.emit(Op::Index);
                    let inner = c.bind(&format!("<pattern item {}>", c.function.locals));
                    c.emit(Op::Store(inner));
                    self.pattern(p, inner, c, failed)?;
                }
            }
            Pattern::Struct { name, fields, .. } => {
                c.emit(Op::Load(value));
                c.emit(match self.struct_identities.get(*name) {
                    Some(identity) => Op::MatchStruct(identity.clone()),
                    None => {
                        return Err(format!(
                            "struct pattern `{name}` has no declaration identity"
                        ));
                    }
                });
                failed.push(c.emit(Op::JumpIfFalse(0)));
                for field in *fields {
                    c.emit(Op::Load(value));
                    c.emit(Op::Field(field.name.into()));
                    let inner = c.bind(&format!("<pattern field {}>", c.function.locals));
                    c.emit(Op::Store(inner));
                    self.pattern(&field.pattern, inner, c, failed)?;
                }
            }
        }
        Ok(())
    }
    fn match_value(
        &mut self,
        scrutinee: &Expr<'a>,
        arms: &[MatchArm<'a>],
        c: &mut Context,
    ) -> Result<()> {
        let outer = c.names.clone();
        let outer_checked = c.checked.clone();
        if self
            .exception_sources
            .contains(&(scrutinee as *const Expr as usize))
        {
            self.capture_exception(scrutinee, "Exception", c)?;
        } else {
            self.expr(scrutinee, c)?;
        }
        let value = c.bind(&format!("<match value {}>", c.function.locals));
        c.emit(Op::Store(value));
        self.match_arms(value, arms, c)?;
        c.names = outer;
        c.checked = outer_checked;
        Ok(())
    }
    fn match_arms(&mut self, value: usize, arms: &[MatchArm<'a>], c: &mut Context) -> Result<()> {
        let outer = c.names.clone();
        let outer_checked = c.checked.clone();
        let mut done = vec![];
        for arm in arms {
            c.names = outer.clone();
            c.checked = outer_checked.clone();
            let mut next = vec![];
            self.pattern(&arm.pattern, value, c, &mut next)?;
            if let Some(guard) = &arm.guard {
                self.expr(guard, c)?;
                next.push(c.emit(Op::JumpIfFalse(0)));
            }
            if arm.bodyless
                && let Expr::EnumConstructor {
                    payload: Some(Expr::Identifier { name, .. }),
                    ..
                } = &arm.body
                && name.starts_with("$__deka_passthrough_")
            {
                c.emit(Op::Load(value));
                c.emit(Op::Field("value".into()));
                let slot = c.bind(name);
                c.emit(Op::Store(slot));
            }
            self.expr(&arm.body, c)?;
            if arm.bodyless {
                // The checker marks a bodyless forwarding arm as never: it
                // returns its reconstructed failure from the enclosing function.
                c.emit(Op::Return);
            } else {
                done.push(c.emit(Op::Jump(0)));
            }
            for jump in next {
                c.patch(jump);
            }
        }
        // The checker proves coverage. Retain a fail-closed fallback for
        // malformed bytecode or host values that violate their declared type.
        c.emit(Op::Const(Literal::String("non-exhaustive match".into())));
        c.emit(Op::Panic);
        for jump in done {
            c.patch(jump);
        }
        c.names = outer;
        c.checked = outer_checked;
        Ok(())
    }
    /// Only the authored subject is guarded. Arm bodies run after the handler
    /// has been removed, so a handler's Throw propagates to its caller.
    fn capture_exception(
        &mut self,
        subject: &Expr<'a>,
        channel: &str,
        c: &mut Context,
    ) -> Result<()> {
        let handler = c.emit(Op::Handler(0));
        c.handler_depth += 1;
        self.expr(subject, c)?;
        c.handler_depth -= 1;
        c.emit(Op::EndHandler);
        c.emit(Op::Enum {
            name: channel.into(),
            case: "Ok".into(),
            index: 0,
            payload: true,
        });
        let end = c.emit(Op::Jump(0));
        c.patch(handler);
        c.emit(Op::Enum {
            name: channel.into(),
            case: if channel == "Result" { "Err" } else { "Throw" }.into(),
            index: 1,
            payload: true,
        });
        c.patch(end);
        Ok(())
    }
    fn unwrap_binding(
        &mut self,
        name: &str,
        scrutinee: &Expr<'a>,
        alternative: &UnwrapAlternative<'a>,
        c: &mut Context,
    ) -> Result<()> {
        let outer = c.names.clone();
        let outer_checked = c.checked.clone();
        self.expr(scrutinee, c)?;
        let value = c.bind(&format!("<unwrap value {}>", c.function.locals));
        c.emit(Op::Store(value));
        c.emit(Op::Load(value));
        c.emit(Op::MatchEnum {
            name: Some("Option".into()),
            case: "Some".into(),
        });
        let result = c.emit(Op::JumpIfFalse(0));
        let some = c.emit(Op::Jump(0));
        c.patch(result);
        c.emit(Op::Load(value));
        c.emit(Op::MatchEnum {
            name: Some("Result".into()),
            case: "Ok".into(),
        });
        let absent = c.emit(Op::JumpIfFalse(0));
        c.patch(some);
        c.emit(Op::Load(value));
        c.emit(Op::Field("value".into()));
        let done = c.emit(Op::Jump(0));
        c.patch(absent);
        match alternative {
            UnwrapAlternative::Block(body) => {
                for (i, statement) in body.iter().enumerate() {
                    if i + 1 == body.len()
                        && let Stmt::Expr { expr, .. } = statement
                    {
                        self.expr(expr, c)?;
                    } else {
                        self.statement(statement, c)?;
                    }
                }
                // Non-value alternatives must exit on every path, verified
                // by the checker. Returns are emitted in this same frame.
            }
            UnwrapAlternative::Match(arms) => self.match_arms(value, arms, c)?,
        }
        c.patch(done);
        c.names = outer;
        c.checked = outer_checked;
        let slot = c.bind(name);
        if !c.loops.is_empty() {
            c.emit(Op::Rebind(slot));
        }
        c.emit(Op::Store(slot));
        Ok(())
    }
    fn statement(&mut self, s: &Stmt<'a>, c: &mut Context) -> Result<()> {
        self.statement_inner(s, c)
            .map_err(|error| self.refusal(s.span(), error))
    }
    fn statement_inner(&mut self, s: &Stmt<'a>, c: &mut Context) -> Result<()> {
        match s {
            Stmt::Import { .. } | Stmt::Empty { .. } => {}
            // Types are erased at run time: an alias is the same value, a
            // newtype is the same value the typechecker keeps apart, and an
            // opaque name has no construction surface. A struct declaration is
            // a record shape; literals consult it for defaults (see the
            // pre-scan in lower_module), the declaration itself emits
            // nothing.
            Stmt::TypeAlias { .. }
            | Stmt::Newtype { .. }
            | Stmt::Opaque { .. }
            | Stmt::Interface { .. }
            | Stmt::Struct { .. } => {}
            // Export groups lower no code; the export-collection pass either
            // erases them (type-only names) or rejects them (deka#1210).
            Stmt::Export {
                decl: ExportDecl::NamedGroup { .. },
                ..
            } => {}
            Stmt::Const { name, value, .. }
            | Stmt::Let { name, value, .. }
            | Stmt::Export {
                decl: ExportDecl::Const { name, value, .. },
                ..
            } => {
                self.expr(value, c)?;
                let slot = c.bind(name);
                // A declaration inside a loop runs once per iteration; give it
                // a fresh cell each time so a closure created this turn keeps
                // this turn's value instead of aliasing the next turn's.
                if !c.loops.is_empty() {
                    c.emit(Op::Rebind(slot));
                }
                c.emit(Op::Store(slot));
            }
            Stmt::UnwrapLet {
                name,
                scrutinee,
                alternative,
                ..
            } => self.unwrap_binding(name, scrutinee, alternative, c)?,
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
            Stmt::Try {
                body,
                catch_name,
                catch_type,
                catch_body,
                ..
            } => {
                let outer = c.names.clone();
                let outer_checked = c.checked.clone();
                let handler = c.emit(Op::Handler(0));
                c.handler_depth += 1;
                self.scoped(body, c)?;
                c.handler_depth -= 1;
                c.emit(Op::EndHandler);
                let end = c.emit(Op::Jump(0));
                c.patch(handler);
                let slot = c.bind(catch_name);
                if !c.loops.is_empty() {
                    c.emit(Op::Rebind(slot));
                }
                c.emit(Op::Store(slot));
                if let Some(ty) = catch_type {
                    let name = self
                        .catch_types
                        .get(&(ty as *const Type as usize))
                        .ok_or("typed catch has no checked constructor")?;
                    if !self.struct_identities.contains_key(name) {
                        return Err("host error constructors in typed catches require native host error support".into());
                    }
                    c.emit(Op::Load(slot));
                    c.emit(Op::MatchStruct(self.struct_identities[name].clone()));
                    let rethrow = c.emit(Op::JumpIfFalse(0));
                    self.scoped(catch_body, c)?;
                    let handled = c.emit(Op::Jump(0));
                    c.patch(rethrow);
                    c.emit(Op::Load(slot));
                    c.emit(Op::Throw);
                    c.patch(handled);
                } else {
                    self.scoped(catch_body, c)?;
                }
                c.patch(end);
                c.names = outer;
                c.checked = outer_checked;
            }
            Stmt::For {
                init,
                condition,
                step,
                body,
                ..
            } => {
                let outer = c.names.clone();
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
                c.loops.push(LoopTargets {
                    handler_depth: c.handler_depth,
                    ..Default::default()
                });
                self.scoped(body, c)?;
                let step_start = c.function.code.len();
                if let Some(step) = step {
                    self.expr(step, c)?;
                    c.emit(Op::Pop);
                }
                c.emit(Op::Jump(start));
                c.patch(end);
                c.finish_loop(step_start);
                c.names = outer;
            }
            Stmt::ForOf {
                name,
                iterable,
                body,
                ..
            } => {
                let outer = c.names.clone();
                let list = c.bind(&format!("<for-of list {}>", c.function.locals));
                let index = c.bind(&format!("<for-of index {}>", c.function.locals));
                self.expr(iterable, c)?;
                c.emit(Op::Store(list));
                c.emit(Op::Const(Literal::Number(0.)));
                c.emit(Op::Store(index));
                let start = c.function.code.len();
                c.emit(Op::Load(index));
                c.emit(Op::Load(list));
                c.emit(Op::Field("length".into()));
                c.emit(Op::Less);
                let end = c.emit(Op::JumpIfFalse(0));
                c.loops.push(LoopTargets {
                    handler_depth: c.handler_depth,
                    ..Default::default()
                });
                let item = c.bind(name);
                c.emit(Op::Rebind(item));
                c.emit(Op::Load(list));
                c.emit(Op::Load(index));
                c.emit(Op::Index);
                c.emit(Op::Store(item));
                self.scoped(body, c)?;
                let step_start = c.function.code.len();
                c.emit(Op::Load(index));
                c.emit(Op::Const(Literal::Number(1.)));
                c.emit(Op::Add);
                c.emit(Op::Store(index));
                c.emit(Op::Jump(start));
                c.patch(end);
                c.finish_loop(step_start);
                c.names = outer;
            }
            Stmt::TupleBinding { names, value, .. } => {
                let temp = c.bind(&format!("<destructure {}>", c.function.locals));
                self.expr(value, c)?;
                c.emit(Op::Store(temp));
                for (i, name) in names.iter().enumerate() {
                    c.emit(Op::Load(temp));
                    c.emit(Op::Const(Literal::Number(i as f64)));
                    c.emit(Op::Index);
                    let slot = c.bind(name);
                    if !c.loops.is_empty() {
                        c.emit(Op::Rebind(slot));
                    }
                    c.emit(Op::Store(slot));
                }
            }
            Stmt::Break { .. } => {
                if c.loops.is_empty() {
                    return Err("break outside a loop".into());
                }
                for _ in c.loops.last().unwrap().handler_depth..c.handler_depth {
                    c.emit(Op::EndHandler);
                }
                let jump = c.emit(Op::Jump(0));
                c.loops.last_mut().unwrap().breaks.push(jump);
            }
            Stmt::Continue { .. } => {
                if c.loops.is_empty() {
                    return Err("continue outside a loop".into());
                }
                for _ in c.loops.last().unwrap().handler_depth..c.handler_depth {
                    c.emit(Op::EndHandler);
                }
                let jump = c.emit(Op::Jump(0));
                c.loops.last_mut().unwrap().continues.push(jump);
            }
            // A receiver method is a plain function whose first parameter is
            // the receiver; calls rewrite to it statically (see the
            // method_calls table filled in lower_module). The receiver is a
            // handle, so a `mut` method's field writes already land on the
            // shared value.
            Stmt::ReceiverMethod {
                receiver_type,
                receiver_name,
                name,
                type_params,
                params,
                body,
                is_async,
                span,
                ..
            } => {
                let mangled = deka_syntax::mangle_method_name(name, receiver_type);
                let mut all = Vec::with_capacity(params.len() + 1);
                all.push(Param {
                    binding: ParamBinding::Identifier(receiver_name),
                    ty: None,
                    default_value: None,
                    span: *span,
                });
                all.extend(params.iter().cloned());
                self.named_function(&mangled, &all, type_params, body, *is_async, c)?;
            }
            // An enum declaration interns each payload-free case as one
            // record `{name, index}` bound to `Enum$Case`; payload cases
            // build their record at each construction site.
            Stmt::Enum { name, cases, .. } => {
                for (index, case) in cases.iter().enumerate() {
                    if case.payload.is_some() {
                        continue;
                    }
                    c.emit(Op::Enum {
                        name: (*name).into(),
                        case: case.name.into(),
                        index,
                        payload: false,
                    });
                    let slot = c.bind(&format!("{}${}", name, case.name));
                    if !c.loops.is_empty() {
                        c.emit(Op::Rebind(slot));
                    }
                    c.emit(Op::Store(slot));
                }
            }
            Stmt::Summon { .. } => {
                return Err(
                    "summoned foreign declarations are not supported by the native VM".into(),
                );
            }
            Stmt::BridgeDecl { .. } => {
                return Err(
                    "ambient bridge declarations are not supported by the native VM".into(),
                );
            }
            Stmt::Export { .. } => {
                return Err(
                    "ambient export declarations are not supported by the native VM".into(),
                );
            }
        }
        Ok(())
    }
    fn jsx_children(&mut self, values: &[Expr<'a>], c: &mut Context) -> Result<usize> {
        let mut children = 0;
        for child in values {
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
                _ => self.thunk("<ui binding>", child, c)?,
            }
            children += 1;
        }
        Ok(children)
    }
    fn reduce(&mut self, receiver: &Expr<'a>, callback: &Expr<'a>, c: &mut Context) -> Result<()> {
        let array = c.bind(&format!("<reduce array {}>", c.function.locals));
        let callback_slot = c.bind(&format!("<reduce callback {}>", c.function.locals));
        let length = c.bind(&format!("<reduce length {}>", c.function.locals));
        let accumulator = c.bind(&format!("<reduce accumulator {}>", c.function.locals));
        let index = c.bind(&format!("<reduce index {}>", c.function.locals));
        self.expr(receiver, c)?;
        c.emit(Op::Store(array));
        self.expr(callback, c)?;
        c.emit(Op::Store(callback_slot));
        // A record field or interface method named reduce keeps ordinary
        // dispatch. Evaluate its receiver and argument only once too.
        c.emit(Op::Load(array));
        c.emit(Op::MatchType(crate::TypeDescriptor::new("array", "Array")));
        let record = c.emit(Op::JumpIfFalse(0));
        c.emit(Op::Load(array));
        c.emit(Op::Field("length".into()));
        c.emit(Op::Store(length));
        c.emit(Op::Load(length));
        c.emit(Op::Const(Literal::Number(0.)));
        c.emit(Op::Equal);
        let nonempty = c.emit(Op::JumpIfFalse(0));
        c.emit(Op::Const(Literal::String(
            "cannot reduce an empty array without an initial value".into(),
        )));
        c.emit(Op::Panic);
        c.patch(nonempty);
        c.emit(Op::Load(array));
        c.emit(Op::Const(Literal::Number(0.)));
        c.emit(Op::Index);
        c.emit(Op::Store(accumulator));
        c.emit(Op::Const(Literal::Number(1.)));
        c.emit(Op::Store(index));
        let start = c.function.code.len();
        c.emit(Op::Load(index));
        c.emit(Op::Load(length));
        c.emit(Op::Less);
        let end = c.emit(Op::JumpIfFalse(0));
        // Capture the initial length, but read each still-present element
        // when visited: callback mutations do not extend this traversal.
        c.emit(Op::Load(array));
        c.emit(Op::Load(index));
        c.emit(Op::ListHas);
        let missing = c.emit(Op::JumpIfFalse(0));
        c.emit(Op::Load(callback_slot));
        c.emit(Op::Load(accumulator));
        c.emit(Op::Load(array));
        c.emit(Op::Load(index));
        c.emit(Op::Index);
        c.emit(Op::Call(2));
        c.emit(Op::Store(accumulator));
        c.patch(missing);
        c.emit(Op::Load(index));
        c.emit(Op::Const(Literal::Number(1.)));
        c.emit(Op::Add);
        c.emit(Op::Store(index));
        c.emit(Op::Jump(start));
        c.patch(end);
        c.emit(Op::Load(accumulator));
        let done = c.emit(Op::Jump(0));
        c.patch(record);
        c.emit(Op::Load(array));
        c.emit(Op::Load(callback_slot));
        c.emit(Op::MethodCall {
            name: "reduce".into(),
            argc: 1,
        });
        c.patch(done);
        Ok(())
    }
    fn host_closure(&mut self, operation: &str, c: &mut Context) -> Result<()> {
        let argc = *self
            .host_arities
            .get(operation)
            .ok_or("unknown host signature")?;
        let mut code = (0..argc).map(Op::Load).collect::<Vec<_>>();
        code.push(Op::Host {
            operation: operation.into(),
            arguments: argc,
        });
        code.push(Op::Return);
        let function = self.functions.len();
        self.functions.push(Function {
            name: operation.into(),
            parameters: argc,
            captures: 0,
            locals: argc,
            asynchronous: false,
            code,
        });
        c.emit(Op::Closure {
            function,
            captures: vec![],
        });
        Ok(())
    }
    fn expr(&mut self, e: &Expr<'a>, c: &mut Context) -> Result<()> {
        self.expr_inner(e, c)
            .map_err(|error| self.refusal(e.span(), error))
    }
    fn expr_inner(&mut self, e: &Expr<'a>, c: &mut Context) -> Result<()> {
        match self
            .exception_forms
            .get(&(e as *const Expr as usize))
            .copied()
        {
            Some(ExceptionEmit::Ok | ExceptionEmit::Throw) => {
                let Expr::EnumConstructor {
                    payload: Some(payload),
                    ..
                } = e
                else {
                    return Err("checked Exception constructor lacks a payload".into());
                };
                self.expr(payload, c)?;
                if self.exception_forms[&(e as *const Expr as usize)] == ExceptionEmit::Throw {
                    c.emit(Op::Throw);
                }
                return Ok(());
            }
            Some(ExceptionEmit::ToResult) => {
                let Expr::Call {
                    callee: Expr::FieldAccess { object, .. },
                    ..
                } = e
                else {
                    return Err("checked Exception conversion has an invalid call".into());
                };
                return self.capture_exception(object, "Result", c);
            }
            Some(ExceptionEmit::FromResult) => {
                let Expr::Call { args, .. } = e else {
                    return Err("checked Exception.from has an invalid call".into());
                };
                self.expr(&args[0], c)?;
                c.emit(Op::Dup);
                c.emit(Op::MatchEnum {
                    name: Some("Result".into()),
                    case: "Ok".into(),
                });
                let err = c.emit(Op::JumpIfFalse(0));
                c.emit(Op::Field("value".into()));
                let end = c.emit(Op::Jump(0));
                c.patch(err);
                c.emit(Op::Field("value".into()));
                c.emit(Op::Throw);
                c.patch(end);
                return Ok(());
            }
            _ => {}
        }
        match e {
            Expr::JsxElement { element, .. } => {
                if element.tag == "slot" {
                    if !element.attributes.is_empty() || element.children.iter().any(|child| {
                        !matches!(child, Expr::JsxText { value, .. } if value.trim().is_empty())
                    }) {
                        return Err("default slot accepts no attributes or nested content".into());
                    }
                    c.emit(Op::Slot);
                    return Ok(());
                }
                if element.tag.chars().next().is_some_and(char::is_uppercase) {
                    // A component imported across an import cycle reads
                    // through the same checked load as any other binding.
                    c.emit_load(element.tag)?;
                    let mut names = vec![];
                    for attr in element.attributes {
                        match &attr.value {
                            Some(value) => self.thunk("<component prop>", value, c)?,
                            // A bare attribute means `true`.
                            None => self.thunk(
                                "<component prop>",
                                &Expr::Boolean {
                                    value: true,
                                    span: attr.span,
                                },
                                c,
                            )?,
                        }
                        names.push(attr.name.into());
                    }
                    if !element.children.is_empty() {
                        if names.iter().any(|name| name == "children") {
                            return Err("component children cannot be supplied both as a prop and nested markup".into());
                        }
                        let children = self.jsx_children(element.children, c)?;
                        c.emit(Op::List(children));
                        names.push("children".into());
                    }
                    c.emit(Op::Props(names));
                    c.emit(Op::ComponentCall);
                    return Ok(());
                }
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
                        && !matches!(value, Expr::Function { .. })
                    {
                        return Err("VM UI event handlers must be function literals".into());
                    }
                    if matches!(attr.name, "className" | "value" | "placeholder")
                        && !matches!(value, Expr::String { .. })
                    {
                        self.thunk("<ui attribute>", value, c)?;
                    } else {
                        self.expr(value, c)?;
                    }
                    names.push(attr.name.into());
                }
                let children = self.jsx_children(element.children, c)?;
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
            Expr::None { .. } => self.enum_constructor("Option", "None", None, c)?,
            Expr::Match {
                scrutinee, arms, ..
            } => self.match_value(scrutinee, arms, c)?,
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
            Expr::Identifier {
                name: "Promise", ..
            } if !c.names.contains_key("Promise") => {
                for kind in PromiseJoin::ALL {
                    let function = self.functions.len();
                    self.functions.push(Function {
                        name: format!("Promise.{}", kind.name()),
                        parameters: 1,
                        captures: 0,
                        locals: 1,
                        asynchronous: false,
                        code: vec![Op::Load(0), Op::PromiseJoin(kind), Op::Return],
                    });
                    c.emit(Op::Closure {
                        function,
                        captures: vec![],
                    });
                }
                c.emit(Op::Record(
                    PromiseJoin::ALL
                        .into_iter()
                        .map(|kind| kind.name().to_owned())
                        .collect(),
                ));
            }
            Expr::Identifier { name, .. }
                if !c.names.contains_key(*name) && self.host_values.contains_key(*name) =>
            {
                c.emit(Op::Host {
                    operation: self.host_values[*name].clone(),
                    arguments: 0,
                });
            }
            Expr::Identifier { name, .. }
                if !c.names.contains_key(*name) && self.host_namespaces.contains_key(*name) =>
            {
                let fields = self.host_namespaces[*name].clone();
                for (_, operation) in &fields {
                    self.host_closure(operation, c)?;
                }
                c.emit(Op::Record(
                    fields.into_iter().map(|(field, _)| field).collect(),
                ));
            }
            Expr::Identifier { name, .. } => {
                if !c.names.contains_key(*name)
                    && let Some(operation) = self.hosts.get(*name)
                {
                    self.host_closure(&operation.clone(), c)?;
                } else {
                    c.emit_load(name)?;
                }
            }
            Expr::Paren { expr, .. } | Expr::Safe { expr, .. } => self.expr(expr, c)?,
            Expr::Binary {
                left, op, right, ..
            } => {
                if matches!(
                    op,
                    BinOp::Assign
                        | BinOp::AddAssign
                        | BinOp::SubAssign
                        | BinOp::MulAssign
                        | BinOp::DivAssign
                        | BinOp::ModAssign
                ) {
                    match left {
                        Expr::Identifier { name, .. } => {
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
                                BinOp::DivAssign => {
                                    c.emit(Op::Div);
                                }
                                BinOp::ModAssign => {
                                    c.emit(Op::Mod);
                                }
                                _ => {}
                            }
                            c.emit(Op::Dup);
                            c.emit(Op::Store(slot));
                        }
                        // Field and element assignment mutate the heap value,
                        // so every alias observes the change. Const-ness is
                        // the typechecker's guard; compound assignment stays
                        // binding-only.
                        Expr::FieldAccess { object, field, .. } if *op == BinOp::Assign => {
                            self.expr(object, c)?;
                            self.expr(right, c)?;
                            c.emit(Op::FieldSet((*field).into()));
                        }
                        Expr::IndexAccess { object, index, .. } if *op == BinOp::Assign => {
                            self.expr(object, c)?;
                            self.expr(index, c)?;
                            self.expr(right, c)?;
                            c.emit(Op::IndexSet);
                        }
                        _ => {
                            return Err(
                                "only binding, field and element assignment are supported".into()
                            );
                        }
                    }
                } else if *op == BinOp::And {
                    self.expr(left, c)?;
                    let short = c.emit(Op::JumpIfFalse(0));
                    self.expr(right, c)?;
                    let end = c.emit(Op::Jump(0));
                    c.patch(short);
                    c.emit(Op::Const(Literal::Bool(false)));
                    c.patch(end);
                } else if *op == BinOp::Or {
                    self.expr(left, c)?;
                    let try_right = c.emit(Op::JumpIfFalse(0));
                    c.emit(Op::Const(Literal::Bool(true)));
                    let end = c.emit(Op::Jump(0));
                    c.patch(try_right);
                    self.expr(right, c)?;
                    c.patch(end);
                } else if *op == BinOp::Pipe {
                    self.pipe(left, right, c)?;
                } else {
                    self.expr(left, c)?;
                    self.expr(right, c)?;
                    c.emit(match op {
                        BinOp::Add => Op::Add,
                        BinOp::Sub => Op::Sub,
                        BinOp::Mul => Op::Mul,
                        BinOp::Div => Op::Div,
                        BinOp::Mod => Op::Mod,
                        BinOp::Lt => Op::Less,
                        BinOp::Le => Op::LessEq,
                        BinOp::Gt => Op::Greater,
                        BinOp::Ge => Op::GreaterEq,
                        BinOp::Eq => Op::Equal,
                        BinOp::Ne => Op::NotEqual,
                        BinOp::BitAnd => Op::BitAnd,
                        BinOp::BitOr => Op::BitOr,
                        BinOp::BitXor => Op::BitXor,
                        BinOp::Shl => Op::Shl,
                        BinOp::Shr => Op::Shr,
                        _ => return Err(format!("operator {op:?} unsupported")),
                    });
                }
            }
            Expr::Call { callee, args, .. } => {
                if self.json_call(e, c)? {
                    return Ok(());
                }
                if let Expr::FieldAccess { object, .. } = callee {
                    let site = e as *const Expr as usize;
                    if self.type_of_calls.contains(&site) {
                        self.expr(object, c)?;
                        c.emit(Op::GetType);
                        return Ok(());
                    }
                    if let Some(descriptor) = self.signature_calls.get(&site).cloned() {
                        self.expr(object, c)?;
                        c.emit(Op::Pop);
                        c.emit(Op::Descriptor(descriptor));
                        return Ok(());
                    }
                }
                // Console output methods share a typed catalog.
                // The checker already guards their printable arguments;
                // all formatting happens in the shared VM ToString operation.
                if let Expr::FieldAccess {
                    object,
                    field: method,
                    ..
                } = callee
                    && matches!(
                        object,
                        Expr::Identifier {
                            name: "console",
                            ..
                        }
                    )
                    && !c.names.contains_key("console")
                    && let Some(operation) = deka_syntax::console::output_operation(method)
                {
                    if !self.console_outputs.contains(operation) {
                        return Err(format!(
                            "console.{method} requires a registered {operation}(string) output operation"
                        ));
                    }
                    c.emit(Op::Const(Literal::String(String::new())));
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            c.emit(Op::Const(Literal::String(" ".into())));
                            c.emit(Op::Add);
                        }
                        self.expr(arg, c)?;
                        c.emit(Op::ToString);
                        c.emit(Op::Add);
                    }
                    c.emit(Op::Host {
                        operation: operation.into(),
                        arguments: 1,
                    });
                    return Ok(());
                }
                // Explicit type arguments erase; the typechecker has already
                // verified them.
                if let Expr::FieldAccess { object, field, .. } = callee
                    && self
                        .number_math_calls
                        .contains(&(e as *const Expr as usize))
                {
                    let method = deka_syntax::math_catalog::method(field)
                        .ok_or("unknown number math method")?;
                    let operation = crate::builtin_math::operation(method.name);
                    if !self.host_arities.contains_key(&operation) {
                        return Err(format!(
                            "native number math operation {} is not registered",
                            method.name
                        ));
                    }
                    self.expr(object, c)?;
                    for argument in *args {
                        self.expr(argument, c)?;
                    }
                    c.emit(Op::Host {
                        operation,
                        arguments: 1 + method.arguments,
                    });
                    return Ok(());
                }
                // A recorded receiver-method call rewrites to its free
                // function: `r.move()` is `move$Mover(r.Mover)` — the embed
                // path walks from the receiver value to the record the
                // method was declared on.
                if let Expr::FieldAccess { object, .. } = callee
                    && let Some((mangled, embed_path)) =
                        self.method_calls.get(&(e as *const Expr as usize)).cloned()
                {
                    let host_method = self
                        .hosts
                        .get(&mangled)
                        .filter(|_| !c.names.contains_key(&mangled))
                        .cloned();
                    if host_method.is_none() {
                        c.emit_load(&mangled)?;
                    }
                    self.expr(object, c)?;
                    for step in &embed_path {
                        c.emit(Op::FieldOrSelf(step.clone()));
                    }
                    for arg in *args {
                        self.expr(arg, c)?;
                    }
                    if let Some(operation) = host_method {
                        c.emit(Op::Host {
                            operation,
                            arguments: 1 + args.len(),
                        });
                    } else {
                        c.emit(Op::Call(1 + args.len()));
                    }
                    return Ok(());
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
                    field: "reduce",
                    ..
                } = callee
                    && let [callback] = *args
                {
                    self.reduce(object, callback, c)?;
                    return Ok(());
                }
                // Mutating list built-ins lower to one in-place op each; the
                // typechecker has already confined them to `let` lists.
                if let Expr::FieldAccess { object, field, .. } = callee
                    && let Some(kind) = match *field {
                        "push" => Some((ListMut::Push, 1)),
                        "pop" => Some((ListMut::Pop, 0)),
                        "shift" => Some((ListMut::Shift, 0)),
                        "unshift" => Some((ListMut::Unshift, 1)),
                        "splice" => Some((ListMut::Splice, 2)),
                        "sort" => Some((ListMut::Sort, 0)),
                        "reverse" => Some((ListMut::Reverse, 0)),
                        "fill" => Some((ListMut::Fill, 2)),
                        "copyWithin" => Some((ListMut::CopyWithin, 2)),
                        _ => None,
                    }
                {
                    let (kind, arity) = kind;
                    if args.len() != arity {
                        return Err(format!("{field} takes {arity} argument(s)"));
                    }
                    self.expr(object, c)?;
                    for arg in *args {
                        self.expr(arg, c)?;
                    }
                    c.emit(Op::ListMut(kind));
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
                if let Expr::FieldAccess {
                    object: Expr::Identifier { name, .. },
                    field,
                    ..
                } = callee
                    && self.enums.contains_key(*name)
                {
                    self.enum_constructor(name, field, args.first(), c)?;
                    return Ok(());
                }
                let unbound = if let Expr::Identifier { name, .. } = callee {
                    (!c.names.contains_key(*name)).then_some(*name)
                } else {
                    None
                };
                if let Some(name) = unbound {
                    if self.newtypes.contains(name) {
                        let [arg] = *args else {
                            return Err(format!("{name} takes exactly one argument"));
                        };
                        self.expr(arg, c)?;
                        c.emit(Op::Newtype(name.into()));
                        return Ok(());
                    }
                    if self.structs.contains_key(name) {
                        // The struct name called as a constructor takes the
                        // record whole: `Person({name: "Ada"})`.
                        let [arg] = *args else {
                            return Err(format!("{name} takes exactly one record argument"));
                        };
                        self.expr(arg, c)?;
                        self.attach_methods(name, c)?;
                        return Ok(());
                    }
                    if name == "unboxNumber" {
                        let [arg] = *args else {
                            return Err("unboxNumber takes exactly one argument".into());
                        };
                        self.expr(arg, c)?;
                        c.emit(Op::ToNumber);
                        return Ok(());
                    }
                    let conversion = match name {
                        "string" => Some(Op::ToString),
                        "toNumber" => Some(Op::ToNumber),
                        "panic" => Some(Op::Panic),
                        _ => None,
                    };
                    if let Some(op) = conversion {
                        let [arg] = *args else {
                            return Err(format!("{name} takes exactly one argument"));
                        };
                        self.expr(arg, c)?;
                        c.emit(op);
                        return Ok(());
                    }
                }
                let host = unbound.and_then(|name| self.hosts.get(name).cloned());
                if let Some(operation) = host {
                    for arg in *args {
                        self.expr(arg, c)?;
                    }
                    c.emit(Op::Host {
                        operation,
                        arguments: args.len(),
                    });
                } else {
                    if let Some(name) = unbound {
                        if self.declared.contains(name) {
                            return Err(format!(
                                "binding {name} is unavailable here; forward references are unsupported"
                            ));
                        }
                        return Err(format!("unknown built-in {name}"));
                    }
                    // A member call the typechecker did not resolve
                    // statically dispatches at run time: an interface-typed
                    // receiver finds its attached `$method`, a record field
                    // holding a function is called plainly.
                    if let Expr::FieldAccess { object, field, .. } = callee {
                        self.expr(object, c)?;
                        for arg in *args {
                            self.expr(arg, c)?;
                        }
                        c.emit(Op::MethodCall {
                            name: (*field).into(),
                            argc: args.len(),
                        });
                        return Ok(());
                    }
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
                if elements.iter().any(|e| matches!(e, Expr::Spread { .. })) {
                    c.emit(Op::List(0));
                    for e in *elements {
                        match e {
                            Expr::Spread { expr, .. } => {
                                self.expr(expr, c)?;
                                c.emit(Op::ListExtend);
                            }
                            _ => {
                                self.expr(e, c)?;
                                c.emit(Op::ListAppend);
                            }
                        }
                    }
                } else {
                    for e in *elements {
                        self.expr(e, c)?;
                    }
                    c.emit(Op::List(elements.len()));
                }
            }
            Expr::Object { fields, .. } => {
                if fields.iter().any(|f| f.key.is_empty()) {
                    c.emit(Op::Record(vec![]));
                    for f in *fields {
                        self.expr(&f.value, c)?;
                        if f.key.is_empty() {
                            c.emit(Op::RecordExtend);
                        } else {
                            c.emit(Op::Record(vec![f.key.to_string()]));
                            c.emit(Op::RecordExtend);
                        }
                    }
                } else {
                    let mut names = vec![];
                    for f in *fields {
                        self.expr(&f.value, c)?;
                        names.push(f.key.to_string());
                    }
                    c.emit(Op::Record(names));
                }
            }
            Expr::StructLiteral { name, fields, .. } => {
                // Evaluate supplied fields once in source order, then build
                // canonical embedded values from the promoted field slots.
                let mut supplied = vec![];
                for f in *fields {
                    self.expr(&f.value, c)?;
                    let slot = c.bind(&format!("<struct field {}>", c.function.locals));
                    c.emit(Op::Store(slot));
                    supplied.push((f.name.to_string(), slot));
                }
                self.struct_value(name, &supplied, c)?;
            }
            Expr::FieldAccess { object, field, .. } => {
                if let Some(operation) = self
                    .native_property_calls
                    .get(&(e as *const Expr as usize))
                    .cloned()
                {
                    self.expr(object, c)?;
                    c.emit(Op::Host {
                        operation,
                        arguments: 1,
                    });
                    return Ok(());
                }
                // A checked namespace case shares construction with constructor syntax.
                if let Expr::Identifier { name, .. } = object
                    && !c.names.contains_key(*name)
                    && self.enum_cases(name).is_some()
                {
                    self.enum_constructor(name, field, None, c)?;
                    return Ok(());
                }
                self.expr(object, c)?;
                c.emit(
                    if self
                        .optional_field_reads
                        .contains(&(e as *const Expr as usize))
                    {
                        Op::OptionalField((*field).into())
                    } else {
                        Op::Field((*field).into())
                    },
                );
            }
            Expr::EnumConstructor {
                enum_name,
                case_name,
                payload,
                ..
            } => {
                self.enum_constructor(enum_name, case_name, payload.as_deref(), c)?;
            }
            Expr::IndexAccess { object, index, .. } => {
                self.expr(object, c)?;
                self.expr(index, c)?;
                c.emit(Op::Index);
            }
            Expr::Unary { op, operand, .. } => {
                self.expr(operand, c)?;
                match op {
                    UnOp::Neg => {
                        c.emit(Op::Neg);
                    }
                    UnOp::Not => {
                        c.emit(Op::Not);
                    }
                    UnOp::Plus => {}
                }
            }
            Expr::TemplateLiteral { parts, .. } => {
                // Each ${…} part becomes text the way string() does it.
                let mut count = 0;
                for part in *parts {
                    match part {
                        TemplatePart::Text(text) => {
                            // The lexer keeps `\${` verbatim (the old JS
                            // emitter leaned on JavaScript's escape); the VM
                            // consumes the text, so it unescapes here.
                            c.emit(Op::Const(Literal::String(text.replace("\\${", "${"))));
                        }
                        TemplatePart::Expr(expr) => {
                            self.expr(expr, c)?;
                            c.emit(Op::ToString);
                        }
                    }
                    if count > 0 {
                        c.emit(Op::Add);
                    }
                    count += 1;
                }
                if count == 0 {
                    c.emit(Op::Const(Literal::String("".into())));
                }
            }
            Expr::BigInt { .. } => {
                return Err("bigint literals are not supported by the native VM".into());
            }
            Expr::Unsafe { .. } => {
                return Err("unsafe blocks are not supported by the native VM".into());
            }
            Expr::Build { .. } => {
                return Err("build blocks are not supported by the native VM".into());
            }
            Expr::Bridge { kind, action, .. } => {
                return Err(format!(
                    "bridge call {kind}.{action} is not supported by the native VM"
                ));
            }
            Expr::ImportMeta { .. } => {
                return Err("import.meta is not supported by the native VM".into());
            }
            Expr::JsxFragment { .. } => {
                return Err("standalone JSX fragments are not supported by the native VM".into());
            }
            Expr::JsxText { .. } => {
                return Err("standalone JSX text is not supported by the native VM".into());
            }
            Expr::Spread { .. } => {
                return Err(
                    "standalone spread expressions are not supported by the native VM".into(),
                );
            }
        }
        if let Some(name) = self.newtype_results.get(&(e as *const Expr as usize)) {
            c.emit(Op::Newtype(name.clone()));
        }
        Ok(())
    }
}

fn descriptor_summary(tree: &deka_syntax::typeck::DescriptorTree<'_>) -> crate::TypeDescriptor {
    use deka_syntax::typeck::DescriptorTree::*;
    let (kind, name) = match tree {
        Leaf { kind, name } => (*kind, name.clone()),
        Recurse { name } => ("struct", (*name).into()),
        Struct { name, .. } => ("struct", (*name).into()),
        Interface { name } => ("interface", (*name).into()),
        Newtype { name, .. } => ("newtype", (*name).into()),
        Enum { name, .. } => ("enum", (*name).into()),
        Array { elem } => ("array", format!("Array<{}>", descriptor_summary(elem).name)),
        Option { inner } => (
            "option",
            format!("Option<{}>", descriptor_summary(inner).name),
        ),
        Tuple { elements } => (
            "tuple",
            format!(
                "[{}]",
                elements
                    .iter()
                    .map(|e| descriptor_summary(e).name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Union { members } => (
            "union",
            members
                .iter()
                .map(|e| descriptor_summary(e).name)
                .collect::<Vec<_>>()
                .join(" | "),
        ),
    };
    crate::TypeDescriptor::new(kind, crate::host::public_native_name(&name))
}
