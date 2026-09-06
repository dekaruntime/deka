use core::{CommandSpec, Context, FlagSpec, Registry};
use std::fs;
use std::path::{Path, PathBuf};

use crate::compile_helper::compile_or_report;

const COMMAND: CommandSpec = CommandSpec {
    name: "check",
    category: "project",
    summary: "validate a DekaScript or legacy DekaScript source file",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--as-package",
        aliases: &[],
        description: "typecheck a package working tree the way an installed consumer would resolve it",
    });
    registry.add_flag(FlagSpec {
        name: "--single-file",
        aliases: &[],
        description: "typecheck only the requested file without project imports",
    });
}

pub fn cmd(context: &Context) {
    if let Err(err) = run(context) {
        stdio::error("check", &err);
        std::process::exit(1);
    }
}

fn run(context: &Context) -> Result<(), String> {
    if context
        .args
        .flags
        .get("--as-package")
        .copied()
        .unwrap_or(false)
    {
        let dir = context
            .args
            .positionals
            .first()
            .ok_or_else(|| "usage: deka check --as-package <package-directory>".to_string())?;
        return check_as_package(Path::new(dir));
    }

    let input = context
        .args
        .positionals
        .first()
        .ok_or_else(|| "usage: deka check <file.ds>".to_string())?;
    let path = Path::new(input);
    if !is_deka_source_path(path) {
        return Err(format!(
            "DekaScript uses .ds only; migrate '{}' before checking it",
            input
        ));
    }

    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let single_file = context
        .args
        .flags
        .get("--single-file")
        .copied()
        .unwrap_or(false);
    let report = if single_file {
        compile_or_report(&source, input)?
    } else if let Some(project_root) = find_project_root(&context.env.cwd, path) {
        check_project_file(path, &project_root, &context.env.cwd)?;
        crate::compile_helper::CompileReport {
            js: String::new(),
            warnings: Vec::new(),
        }
    } else {
        compile_or_report(&source, input)?
    };

    // Warnings never gate `deka check` -- a program with only warnings is a
    // successful check (deka#59). They're printed with the same colored,
    // span-anchored renderer used for errors so they read as "worth
    // knowing" rather than a pass/fail signal, and are never confusable
    // with a `[fail]` line since we still report success below.
    for warning in &report.warnings {
        eprintln!("{}", warning);
    }

    stdio::success(&format!("checked {}", path.display()));
    Ok(())
}

/// Find the nearest project context for a check target. A project check uses
/// the module graph so imports resolve exactly as they do for build and serve;
/// files outside a project retain the historical standalone behavior.
fn find_project_root(cwd: &Path, input: &Path) -> Option<PathBuf> {
    let absolute_input = if input.is_absolute() {
        input.to_path_buf()
    } else {
        cwd.join(input)
    };
    let start = if absolute_input.is_dir() {
        absolute_input
    } else {
        absolute_input.parent()?.to_path_buf()
    };

    for dir in start.ancestors() {
        if dir.join("deka.json").is_file() || dir.join("deka.lock").is_file() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

fn check_project_file(path: &Path, project_root: &Path, cwd: &Path) -> Result<(), String> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let loader = deka_compile::module_graph::FsModuleLoader::new(project_root.to_path_buf());
    deka_compile::module_graph::compile_module_graph(&absolute_path, &loader)
        .map_err(|diagnostics| deka_compile::format_diagnostics(&diagnostics))?;
    Ok(())
}

/// Typecheck a package working tree with the resolution an installed consumer
/// gets (deka#470). A standalone `deka check index.ds` is neither sound nor
/// complete for this: in-place resolution differs from installed resolution,
/// so five of eight stdlib packages fail standalone for reasons that have
/// nothing to do with publish-worthiness.
///
/// The strategy is to build a minimal scratch consumer, link the package into
/// it with the same machinery as `deka link`, install the package's own
/// registry dependencies, and compile a consumer entry through the real
/// module graph. Every gate an installed consumer hits — project gate, module
/// validation, linked-module precedence — sees exactly this shape.
fn check_as_package(package_dir: &Path) -> Result<(), String> {
    let package_dir = fs::canonicalize(package_dir).map_err(|err| {
        format!(
            "package directory does not exist: {} ({err})",
            package_dir.display()
        )
    })?;

    let manifest_raw = fs::read_to_string(package_dir.join("deka.json"))
        .map_err(|err| format!("cannot read package deka.json: {err}"))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_raw)
        .map_err(|err| format!("package deka.json is invalid: {err}"))?;
    let name = manifest
        .get("name")
        .and_then(|value| value.as_str())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| "package deka.json must contain a non-empty `name`".to_string())?;
    let version = manifest
        .get("version")
        .and_then(|value| value.as_str())
        .unwrap_or("0.0.0");
    let main = manifest
        .get("main")
        .and_then(|value| value.as_str())
        .unwrap_or("index.ds");

    let entry_path = package_dir.join(main);
    let entry_source = fs::read_to_string(&entry_path)
        .map_err(|err| format!("cannot read package entry {}: {err}", entry_path.display()))?;
    let surface = collect_export_surface(&entry_source, main)?;
    if surface.is_empty() {
        return Err(format!(
            "package entry {main} exports nothing; there is no consumer surface to verify"
        ));
    }

    let scratch = tempfile::tempdir()
        .map_err(|err| format!("failed to create scratch consumer project: {err}"))?;
    let project = scratch.path();

    // The consumer declares the package under test plus the package's own
    // dependencies, so the project gates see a normal manifest.
    let mut dependencies = serde_json::Map::new();
    dependencies.insert(
        name.to_string(),
        serde_json::Value::String(version.to_string()),
    );
    let package_deps = manifest
        .get("dependencies")
        .and_then(|deps| deps.as_object());
    if let Some(deps) = package_deps {
        for (dep, dep_version) in deps {
            dependencies.insert(dep.clone(), dep_version.clone());
        }
    }
    let consumer_manifest = serde_json::json!({
        "name": "deka-check-as-package",
        "version": "0.0.0",
        "main": "main.ds",
        "dependencies": dependencies,
    });
    fs::write(
        project.join("deka.json"),
        format!("{}\n", serde_json::to_string_pretty(&consumer_manifest).unwrap()),
    )
    .map_err(|err| format!("failed to write scratch deka.json: {err}"))?;
    fs::write(project.join("deka.lock"), "{\n  \"version\": 1,\n  \"packages\": {}\n}\n")
        .map_err(|err| format!("failed to write scratch deka.lock: {err}"))?;
    // The consumer must actually reference every imported name: an import
    // that is never used is shaken out of the graph before typechecking,
    // which would let a broken package pass the gate. Values are referenced
    // as values; type names appear in annotation position only, since a
    // newtype constructor is module-private by design.
    let mut consumer_main = format!("import {{ {} }} from \"{name}\"\n", surface.all_names().join(", "));
    for (index, value_name) in surface.value_names.iter().enumerate() {
        consumer_main.push_str(&format!("const __check_value_{index} = {value_name}\n"));
    }
    for (index, type_name) in surface.type_names.iter().enumerate() {
        consumer_main.push_str(&format!(
            "fn __check_type_{index}(x: {type_name}) {type_name} {{\n  return x\n}}\n"
        ));
    }
    fs::write(project.join("main.ds"), consumer_main)
        .map_err(|err| format!("failed to write scratch main.ds: {err}"))?;

    // Link the working tree in; resolution prefers the link over anything
    // installed, so the bytes being verified are the ones about to ship.
    pm::link_package_at(project, &package_dir).map_err(|err| err.to_string())?;

    // The package's own dependencies resolve from the registry, exactly as
    // they would for a consumer installing the published version.
    if package_deps.is_some_and(|deps| !deps.is_empty()) {
        let exe = std::env::current_exe()
            .map_err(|err| format!("failed to locate the deka binary: {err}"))?;
        let install = std::process::Command::new(exe)
            .args(["install", "--yes"])
            .current_dir(project)
            .env("DEKA_SECURITY_NO_PROMPT", "1")
            .output()
            .map_err(|err| format!("failed to run `deka install` for package dependencies: {err}"))?;
        if !install.status.success() {
            return Err(format!(
                "installing the package's registry dependencies failed:\n{}",
                String::from_utf8_lossy(&install.stderr)
            ));
        }
    }

    let entry = project.join("main.ds");
    let loader = deka_compile::module_graph::FsModuleLoader::new(project.to_path_buf());
    match deka_compile::module_graph::compile_module_graph(&entry, &loader) {
        Ok(_) => {
            stdio::success(&format!("checked {name} as an installed consumer"));
            Ok(())
        }
        Err(diagnostics) => {
            eprintln!("{}", deka_compile::format_diagnostics(&diagnostics));
            Err(format!(
                "{name} does not typecheck as an installed consumer ({} diagnostic(s))",
                diagnostics.len()
            ))
        }
    }
}

/// The export surface of a package entry, split by how a consumer may
/// reference each name. Type names (structs, enums, aliases, newtypes) are
/// only safe in annotation position — a newtype's constructor is
/// module-private, so `Name(x)` in the consumer is a diagnostic by design.
/// Value names (functions, constants) are safe to reference as values.
struct ExportSurface {
    value_names: Vec<String>,
    type_names: Vec<String>,
}

impl ExportSurface {
    fn is_empty(&self) -> bool {
        self.value_names.is_empty() && self.type_names.is_empty()
    }

    fn all_names(&self) -> Vec<String> {
        self.value_names
            .iter()
            .chain(self.type_names.iter())
            .cloned()
            .collect()
    }
}

/// Collect the export surface of a package entry so the scratch consumer can
/// import it. Type-only exports are legal to import (the checker distinguishes
/// type position from value position), so every exported name is included.
fn collect_export_surface(source: &str, entry_name: &str) -> Result<ExportSurface, String> {
    let arena = bumpalo::Bump::new();
    let parsed = deka_syntax::parse::parse(source, &arena);
    if let Some(error) = parsed
        .errors
        .iter()
        .find(|diagnostic| matches!(diagnostic.severity, deka_syntax::diagnostics::Severity::Error))
    {
        return Err(format!(
            "package entry {entry_name} does not parse: {}:{}: {}",
            error.line, error.column, error.message
        ));
    }
    let program = parsed
        .program
        .ok_or_else(|| format!("package entry {entry_name} does not parse"))?;

    let exports = deka_syntax::collect_module_exports(&program, &arena);
    let value_names = exports
        .values
        .keys()
        .map(|name| (*name).to_string())
        .collect();
    let type_names = exports
        .structs
        .keys()
        .chain(exports.enums.keys())
        .chain(exports.aliases.keys())
        .chain(exports.newtypes.keys())
        .map(|name| (*name).to_string())
        .collect();
    Ok(ExportSurface {
        value_names,
        type_names,
    })
}

fn is_deka_source_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ds") | Some("dsx")
    )
}

#[cfg(test)]
mod tests {
    use super::is_deka_source_path;
    use std::path::Path;

    #[test]
    fn accepts_dekascript_only() {
        assert!(is_deka_source_path(Path::new("main.ds")));
        assert!(is_deka_source_path(Path::new("page.dsx")));
        assert!(!is_deka_source_path(Path::new("main.phpx")));
        assert!(!is_deka_source_path(Path::new("main.ts")));
    }
}
