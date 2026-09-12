use core::Context;
use runtime_core::modules::MODULES_DIR;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::project;
use crate::cli::build_dsc;

pub(super) fn run(context: &Context, input: &str) -> Result<(), String> {
    if !project::is_deka_source_path(Path::new(input)) {
        return Err(format!(
            "DekaScript uses .ds only; migrate '{}' before building it",
            input
        ));
    }

    let input_path = PathBuf::from(input);
    let output_path = resolve_output_path(output_arg(context), &input_path)?;
    if bundle_enabled(context) {
        build_bundle_to_path(&input_path, &output_path, minify_enabled(context))?;
    } else {
        build_to_path(&input_path, &output_path)?;
    }

    stdio::success(&format!(
        "built {} -> {}",
        input_path.display(),
        output_path.display()
    ));
    Ok(())
}

pub(super) fn build_to_path(input_path: &Path, output_path: &Path) -> Result<(), String> {
    let output = build_to_string(input_path)?;
    let js = output.js;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }

    fs::write(output_path, js)
        .map_err(|err| format!("failed to write {}: {}", output_path.display(), err))?;

    let import_map_path = resolve_import_map_path(output_path);
    let import_map = emit_import_map_json(&output.import_paths, output_path);
    fs::write(&import_map_path, import_map)
        .map_err(|err| format!("failed to write {}: {}", import_map_path.display(), err))?;

    Ok(())
}

pub(super) fn build_bundle_to_path(
    input_path: &Path,
    output_path: &Path,
    minify: bool,
) -> Result<(), String> {
    // `build_to_string` validates the entry and enforces project layout
    // (ds_modules present) before dsc resolves the import graph.
    let output = build_to_string(input_path)?;

    // Bundling is dsc's stage (dsc#157): `dsc transpile --bundle` resolves the
    // entry's graph — including `.ds` imports, which it compiles itself — and
    // emits one file. minify maps onto dsc's `--treeshake`.
    let bundle = build_dsc::transpile_bundle(&output.project_root, input_path, minify)?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }

    fs::write(output_path, bundle)
        .map_err(|err| format!("failed to write {}: {}", output_path.display(), err))?;

    Ok(())
}

fn output_arg(context: &Context) -> Option<String> {
    context
        .args
        .params
        .get("--out")
        .or_else(|| context.args.params.get("-o"))
        .or_else(|| context.args.params.get("--outdir"))
        .cloned()
}

fn bundle_enabled(context: &Context) -> bool {
    context.args.flags.get("--bundle").copied().unwrap_or(false)
}

fn minify_enabled(context: &Context) -> bool {
    context.args.flags.get("--minify").copied().unwrap_or(false)
}

fn resolve_output_path(out: Option<String>, input_path: &Path) -> Result<PathBuf, String> {
    if let Some(out) = out {
        return Ok(PathBuf::from(out));
    }

    let stem = input_path
        .file_stem()
        .and_then(|v| v.to_str())
        .ok_or_else(|| format!("invalid input filename: {}", input_path.display()))?;

    Ok(PathBuf::from("dist").join(format!("{}.js", stem)))
}

fn resolve_import_map_path(output_path: &Path) -> PathBuf {
    output_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("importmap.json")
}

fn emit_import_map_json(import_paths: &[String], output_path: &Path) -> String {
    let mut imports = default_import_map();

    for spec in import_paths {
        let spec = spec.trim();
        if !is_bare_specifier(spec) {
            continue;
        }

        if !imports.contains_key(spec) && !is_covered_by_prefix_map(&imports, spec) {
            imports.insert(
                spec.to_string(),
                default_import_target_for(spec, output_path),
            );
        }
    }

    serde_json::to_string_pretty(&serde_json::json!({ "imports": imports }))
        .unwrap_or_else(|_| "{\n  \"imports\": {}\n}".to_string())
}

fn default_import_map() -> BTreeMap<String, String> {
    let mut imports = BTreeMap::from([("@/".to_string(), "/".to_string())]);
    for prefix in runtime_core::module_spec::STDLIB_SPEC_PREFIXES {
        imports.insert((*prefix).to_string(), stdlib_prefix_target(prefix));
    }
    imports
}

/// Import-map target for a stdlib prefix. `deka install` writes every stdlib
/// package under the `@deka` scope (`ds_modules/@deka/<pkg>/`), and the server
/// resolvers know it — they try the scoped alias for every prefixed specifier.
/// The browser has only this map, so the prefix target must be the scoped
/// layout too. Derive it from `module_spec_aliases`, the same helper the
/// server resolvers use to compute the alias, rather than repeating a second
/// literal list that can drift away from where packages actually land
/// (deka#622 finding D).
fn stdlib_prefix_target(prefix: &str) -> String {
    let scoped = runtime_core::module_spec::module_spec_aliases(prefix)
        .into_iter()
        .find(|alias| alias.starts_with("@deka/"))
        .expect("bare stdlib prefixes must carry a @deka alias");
    format!("/{MODULES_DIR}/{scoped}")
}

fn is_bare_specifier(spec: &str) -> bool {
    !spec.is_empty()
        && !spec.starts_with("./")
        && !spec.starts_with("../")
        && !spec.starts_with('/')
        && !spec.starts_with("http://")
        && !spec.starts_with("https://")
}

fn is_covered_by_prefix_map(map: &BTreeMap<String, String>, spec: &str) -> bool {
    map.keys()
        .any(|key| key.ends_with('/') && spec.starts_with(key))
}

fn default_import_target_for(spec: &str, output_path: &Path) -> String {
    let mut rel = String::new();
    let depth = output_path
        .parent()
        .map(|parent| parent.components().count().saturating_sub(1))
        .unwrap_or(0);

    for _ in 0..depth {
        rel.push_str("../");
    }
    rel.push_str("ds_modules/");
    rel.push_str(spec.trim_start_matches('/'));

    if !rel.ends_with(".js") && !rel.ends_with('/') {
        rel.push_str(".js");
    }

    rel
}

struct JsBuildOutput {
    js: String,
    import_paths: Vec<String>,
    project_root: PathBuf,
}

fn build_to_string(input_path: &Path) -> Result<JsBuildOutput, String> {
    let source = fs::read_to_string(input_path)
        .map_err(|err| format!("failed to read {}: {}", input_path.display(), err))?;
    let import_paths = runtime_core::ds_imports::paths(&source);

    // Validate the source before checking project layout so that syntax/type
    // errors are surfaced immediately instead of being blocked by a missing
    // deka.lock or php_modules/ directory (dekaruntime/deka#117).
    let js = build_dsc::transpile_file(input_path)?;

    let project_root = project::resolve_project_root(input_path)?;
    project::ensure_project_layout(&project_root, None, &import_paths)?;

    Ok(JsBuildOutput {
        js,
        import_paths,
        project_root,
    })
}
