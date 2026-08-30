use bundler::{BuildOptions, VirtualSource, bundle_virtual_entry};
use core::{CommandSpec, Context, ParamSpec, Registry};
use runtime_core::module_spec::{ds_source_candidates, module_spec_aliases};
use runtime_core::modules::{resolve_modules_dir, MODULES_DIR};

use crate::compile_helper::compile_js_or_report;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const COMMAND: CommandSpec = CommandSpec {
    name: "build",
    category: "project",
    summary: "build a DekaScript file into a JavaScript module",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(core::FlagSpec {
        name: "--bundle",
        aliases: &[],
        description: "bundle emitted JS into a single file (in-memory, no intermediate files)",
    });
    registry.add_flag(core::FlagSpec {
        name: "--minify",
        aliases: &[],
        description: "minify bundled output (only with --bundle)",
    });
    registry.add_param(ParamSpec {
        name: "--out",
        description: "output JavaScript file path",
    });
}

pub fn cmd(context: &Context) {
    if let Err(err) = run(context) {
        stdio::error("build", &err);
        std::process::exit(1);
    }
}

fn run(context: &Context) -> Result<(), String> {
    if let Some(first) = context.args.positionals.first() {
        if Path::new(first)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("phpx"))
        {
            return Err(format!(
                "DekaScript uses .ds only; migrate '{}' before building it",
                first
            ));
        }
        if is_deka_source_path(Path::new(first)) {
            return run_single_file_build(context, first);
        }
    }

    run_web_project_build(context)
}

fn run_single_file_build(context: &Context, input: &str) -> Result<(), String> {
    if !is_deka_source_path(Path::new(input)) {
        return Err(format!(
            "DekaScript uses .ds only; migrate '{}' before building it",
            input
        ));
    }

    let input_path = PathBuf::from(input);
    let output_path = resolve_output_path(output_arg(context), &input_path)?;
    if bundle_enabled(context) {
        build_single_file_bundle_to_path(
            &input_path,
            &output_path,
            minify_enabled(context),
        )?;
    } else {
        build_single_file_to_path(&input_path, &output_path)?;
    }

    stdio::success(&format!(
        "built {} -> {}",
        input_path.display(),
        output_path.display()
    ));
    Ok(())
}

fn run_web_project_build(context: &Context) -> Result<(), String> {
    let root_hint = context
        .args
        .positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|err| err.to_string())?);

    let project_root = resolve_project_root(&root_hint)?;
    ensure_web_project_layout(&project_root)?;

    let app_dir = project_root.join("app");
    let public_dir = project_root.join("public");
    let entry_path = resolve_web_entry(&project_root)?;
    let entry_source = fs::read_to_string(&entry_path)
        .map_err(|err| format!("failed to read {}: {}", entry_path.display(), err))?;
    let hydration_enabled = has_hydration_component(&entry_source);
    let bundle = bundle_enabled(context);
    let minify = minify_enabled(context);

    // `build` must fail closed: every .ds source under app/ has to compile
    // before we write anything to dist/. Historically this function only
    // ever compiled `entry_path` (and only when hydration was enabled) and
    // otherwise copied app/ into dist/server/app as raw, unvalidated bytes
    // (see copy_dir_recursive below) -- so a project with a syntactically
    // invalid non-entry file, or an invalid entry file with no hydration
    // component, would "build" successfully. Validate everything up front.
    validate_app_dir_sources(&app_dir)?;

    let dist_root = project_root.join("dist");
    let dist_client = dist_root.join("client");
    let dist_server = dist_root.join("server");

    fs::create_dir_all(&dist_client)
        .map_err(|err| format!("failed to create {}: {}", dist_client.display(), err))?;
    fs::create_dir_all(&dist_server)
        .map_err(|err| format!("failed to create {}: {}", dist_server.display(), err))?;

    copy_dir_recursive(&public_dir, &dist_client)?;

    let client_index = dist_client.join("index.html");
    let index_raw = fs::read_to_string(&client_index)
        .map_err(|err| format!("failed to read {}: {}", client_index.display(), err))?;
    let template_html = extract_template_html(&entry_source).unwrap_or_default();
    let with_app = inject_app_html(&index_raw, &template_html);

    let final_index = if hydration_enabled {
        let dist_assets = dist_client.join("assets");
        fs::create_dir_all(&dist_assets)
            .map_err(|err| format!("failed to create {}: {}", dist_assets.display(), err))?;

        let client_js = dist_assets.join("main.js");
        if bundle {
            build_single_file_bundle_to_path(&entry_path, &client_js, minify)?;
        } else {
            build_single_file_to_path(&entry_path, &client_js)?;
        }

        if !bundle {
            let assets_importmap = dist_assets.join("importmap.json");
            let client_importmap = dist_client.join("importmap.json");
            if assets_importmap.is_file() {
                fs::copy(&assets_importmap, &client_importmap).map_err(|err| {
                    format!(
                        "failed to copy {} -> {}: {}",
                        assets_importmap.display(),
                        client_importmap.display(),
                        err
                    )
                })?;
            }
        }

        inject_web_bootstrap_tags(&with_app, true, bundle)
    } else {
        inject_web_bootstrap_tags(&with_app, false, bundle)
    };

    fs::write(&client_index, final_index)
        .map_err(|err| format!("failed to write {}: {}", client_index.display(), err))?;

    copy_dir_recursive(&app_dir, &dist_server.join("app"))?;

    let modules_dir = project_root.join(MODULES_DIR);
    if modules_dir.is_dir() {
        copy_dir_recursive(&modules_dir, &dist_server.join(MODULES_DIR))?;
    }
    for file in ["deka.json", "deka.lock"] {
        let src = project_root.join(file);
        if src.is_file() {
            let dst = dist_server.join(file);
            fs::copy(&src, &dst).map_err(|err| {
                format!(
                    "failed to copy {} -> {}: {}",
                    src.display(),
                    dst.display(),
                    err
                )
            })?;
        }
    }

    stdio::success(&format!(
        "built web project {}\n  client: {}\n  server: {}\n  hydration: {}",
        project_root.display(),
        dist_client.display(),
        dist_server.display(),
        if hydration_enabled {
            "enabled"
        } else {
            "disabled"
        }
    ));
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

fn emit_import_map_json(meta: &deka_compile::SourceModuleMeta, output_path: &Path) -> String {
    let mut imports = default_import_map();

    for decl in &meta.imports {
        let spec = decl.path.trim();
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
    BTreeMap::from([
        ("@/".to_string(), "/".to_string()),
        (
            "component/".to_string(),
            "/ds_modules/component/".to_string(),
        ),
        ("deka/".to_string(), "/ds_modules/deka/".to_string()),
        (
            "encoding/".to_string(),
            "/ds_modules/encoding/".to_string(),
        ),
        ("db/".to_string(), "/ds_modules/db/".to_string()),
    ])
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

fn resolve_project_root(input_path: &Path) -> Result<PathBuf, String> {
    let start = project_root_search_start(input_path);

    let mut nearest_manifest_root = None;
    for dir in start.ancestors() {
        if dir.join("deka.json").is_file() {
            let dir = dir.to_path_buf();
            if dir.join("deka.lock").is_file() {
                return Ok(dir);
            }
            if nearest_manifest_root.is_none() {
                nearest_manifest_root = Some(dir);
            }
        }
    }

    if let Some(root) = nearest_manifest_root {
        return Ok(root);
    }

    Err(format!(
        "deka build requires a deka.json project root (searched from {})",
        input_path.display()
    ))
}

fn project_root_search_start(input_path: &Path) -> PathBuf {
    if input_path.is_dir() {
        return input_path.to_path_buf();
    }

    input_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

fn ensure_project_layout(
    project_root: &Path,
    module_root: Option<&Path>,
    meta: &deka_compile::SourceModuleMeta,
) -> Result<(), String> {
    // When an explicit external module_root is provided, trust it for stdlib
    // modules and skip the local ds_modules/ check. Tenant-local packages still
    // require a lockfile and local ds_modules/.
    if module_root.is_some_and(|root| root != project_root) {
        return Ok(());
    }

    let lock_path = project_root.join("deka.lock");
    if !lock_path.is_file() {
        return Err(format!(
            "deka build requires deka.lock at project root: {}",
            lock_path.display()
        ));
    }

    let stdlib_imports = collect_stdlib_imports(meta);
    if stdlib_imports.is_empty() {
        return Ok(());
    }

    let modules_dir = resolve_modules_dir(project_root);
    if !modules_dir.is_dir() {
        return Err(format!(
            "deka build requires ds_modules/ at project root when using stdlib imports ({}). Run `deka install`.",
            stdlib_imports.join(", ")
        ));
    }

    let mut missing = Vec::new();
    for spec in stdlib_imports {
        if resolve_module_file(&modules_dir, &spec).is_none() {
            missing.push(spec);
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing stdlib modules under {}: {}. Run `deka install`.",
            modules_dir.display(),
            missing.join(", ")
        ))
    }
}

fn collect_stdlib_imports(meta: &deka_compile::SourceModuleMeta) -> Vec<String> {
    let mut seen = BTreeSet::new();
    for decl in &meta.imports {
        let spec = decl.path.trim();
        if is_stdlib_module_spec(spec) {
            seen.insert(spec.to_string());
        }
    }
    seen.into_iter().collect()
}

fn is_stdlib_module_spec(spec: &str) -> bool {
    if !is_bare_specifier(spec) || spec.starts_with("@user/") {
        return false;
    }

    if let Some(rest) = spec.strip_prefix("@deka/") {
        return is_stdlib_module_spec(rest);
    }

    spec.starts_with("component/")
        || spec.starts_with("deka/")
        || spec.starts_with("encoding/")
        || spec.starts_with("db/")
        || matches!(
            spec,
            "json"
                | "postgres"
                | "mysql"
                | "sqlite"
                | "bytes"
                | "buffer"
                | "tcp"
                | "tls"
                | "fs"
                | "crypto"
                | "jwt"
                | "test"
                | "cookies"
                | "auth"
                | "db"
                | "time"
                | "io"
        )
}

fn resolve_module_file(modules_dir: &Path, spec: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for alias in module_spec_aliases(spec) {
        candidates.extend(ds_source_candidates(&modules_dir.join(&alias)));
    }

    // For prefixed stdlib specifiers (e.g. encoding/json) also check the scoped
    // @deka layout — stdlib packages installed via `deka install` live there.
    if spec.contains('/')
        && !spec.starts_with('@')
        && !spec.starts_with("./")
        && !spec.starts_with("../")
    {
        let scoped = format!("@deka/{}", spec);
        candidates.extend(ds_source_candidates(&modules_dir.join(&scoped)));
    }

    candidates.into_iter().find(|path| path.is_file())
}

fn load_deka_json(project_root: &Path) -> Result<serde_json::Value, String> {
    let deka_path = project_root.join("deka.json");
    let raw = fs::read_to_string(&deka_path).map_err(|err| {
        format!(
            "failed to read {}: {}",
            project_root.join("deka.json").display(),
            err
        )
    })?;
    serde_json::from_str(&raw).map_err(|err| {
        format!(
            "invalid {}: {}",
            project_root.join("deka.json").display(),
            err
        )
    })
}

fn ensure_web_project_layout(project_root: &Path) -> Result<(), String> {
    let required_files = [
        project_root.join("deka.json"),
        project_root.join("deka.lock"),
    ];
    for file in &required_files {
        if !file.is_file() {
            return Err(format!("missing required file: {}", file.display()));
        }
    }

    let required_dirs = [project_root.join("app"), project_root.join("public")];
    for dir in &required_dirs {
        if !dir.is_dir() {
            return Err(format!("missing required directory: {}", dir.display()));
        }
    }

    let index = project_root.join("public").join("index.html");
    if !index.is_file() {
        return Err(format!("missing required file: {}", index.display()));
    }

    let json = load_deka_json(project_root)?;
    let project_type = json
        .get("type")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase());

    if project_type.as_deref() != Some("serve") {
        let got = project_type.unwrap_or_else(|| "<missing>".to_string());
        return Err(format!(
            "web build requires deka.json type=\"serve\" (got: {}) at {}",
            got,
            project_root.join("deka.json").display()
        ));
    }

    Ok(())
}

fn resolve_web_entry(project_root: &Path) -> Result<PathBuf, String> {
    let json = load_deka_json(project_root)?;

    let entry = json
        .get("serve")
        .and_then(|v| v.get("entry"))
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "web build requires deka.json serve.entry (example: \"app/main.ds\") in {}",
                project_root.join("deka.json").display()
            )
        })?;

    let entry_path = project_root.join(entry);
    if !entry_path.is_file() {
        return Err(format!(
            "serve.entry points to missing file: {}",
            entry_path.display()
        ));
    }

    let app_dir = project_root.join("app");
    if !entry_path.starts_with(&app_dir) {
        return Err(format!(
            "serve.entry must point inside app/: {}",
            entry_path.display()
        ));
    }

    if !is_deka_source_path(&entry_path) {
        return Err(format!(
            "serve.entry must be a .ds file: {}",
            entry_path.display()
        ));
    }

    Ok(entry_path)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if fs::read_dir(src)
        .map_err(|err| format!("failed to read {}: {}", src.display(), err))?
        .next()
        .is_none()
    {
        return Ok(());
    }
    fs::create_dir_all(dst)
        .map_err(|err| format!("failed to create {}: {}", dst.display(), err))?;
    let entries =
        fs::read_dir(src).map_err(|err| format!("failed to read {}: {}", src.display(), err))?;

    for entry in entries {
        let entry = entry.map_err(|err| format!("read_dir entry error: {}", err))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| format!("file_type error for {}: {}", src_path.display(), err))?;

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
            }
            fs::copy(&src_path, &dst_path).map_err(|err| {
                format!(
                    "failed to copy {} -> {}: {}",
                    src_path.display(),
                    dst_path.display(),
                    err
                )
            })?;
        }
    }

    Ok(())
}

/// Recursively finds every `.ds` file under `app_dir` and compiles it with
/// the same v2 compiler call `deka check` uses, discarding the emitted JS. This is validation only -- dist/server/app still receives
/// the original source bytes via `copy_dir_recursive`, unchanged. The point
/// is solely to make `deka build` fail closed (non-zero exit, no dist/
/// output written) on any source under app/ that the compiler itself would
/// reject, matching what `deka check` already reports for that same file.
fn validate_app_dir_sources(app_dir: &Path) -> Result<(), String> {
    for path in collect_deka_source_files(app_dir)? {
        let input = path
            .to_str()
            .ok_or_else(|| format!("invalid utf-8 path: {}", path.display()))?;
        let source = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        compile_js_or_report(&source, input)
            .map_err(|err| format!("{}: {}", path.display(), err))?;
    }
    Ok(())
}

fn collect_deka_source_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }

    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|err| format!("failed to read {}: {}", current.display(), err))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("read_dir entry error: {}", err))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|err| format!("file_type error for {}: {}", path.display(), err))?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && is_deka_source_path(&path) {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn inject_web_bootstrap_tags(
    index_html: &str,
    hydration_enabled: bool,
    bundle_enabled: bool,
) -> String {
    let import_map_tag = r#"<script type="importmap" src="/importmap.json"></script>"#;
    let module_tag = r#"<script type="module" src="/assets/main.js"></script>"#;

    let mut out = index_html.to_string();

    if !hydration_enabled {
        out = out.replace(import_map_tag, "");
        out = out.replace(module_tag, "");
        return out;
    }

    if bundle_enabled {
        out = out.replace(import_map_tag, "");
    } else if !out.contains(import_map_tag) {
        if out.contains("</head>") {
            out = out.replace("</head>", &format!("  {}\n</head>", import_map_tag));
        } else {
            out.push('\n');
            out.push_str(import_map_tag);
            out.push('\n');
        }
    }

    if !out.contains(module_tag) {
        if out.contains("</body>") {
            out = out.replace("</body>", &format!("  {}\n</body>", module_tag));
        } else {
            out.push('\n');
            out.push_str(module_tag);
            out.push('\n');
        }
    }

    out
}

fn has_hydration_component(source: &str) -> bool {
    source.contains("<Hydration") || source.contains("<Hydration/")
}

fn extract_template_html(_source: &str) -> Option<String> {
    // RFD 24: no frontmatter / trailing template capture.
    None
}

fn inject_app_html(index_html: &str, app_html: &str) -> String {
    if app_html.trim().is_empty() {
        return index_html.to_string();
    }

    let mount = "<div id=\"app\"></div>";
    if index_html.contains(mount) {
        return index_html.replacen(mount, &format!("<div id=\"app\">{}</div>", app_html), 1);
    }

    if index_html.contains("</body>") {
        return index_html.replacen("</body>", &format!("{}\n</body>", app_html), 1);
    }

    let mut out = index_html.to_string();
    out.push('\n');
    out.push_str(app_html);
    out
}

struct JsBuildOutput {
    js: String,
    meta: deka_compile::SourceModuleMeta,
    project_root: PathBuf,
}

fn build_single_file_to_path(
    input_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    let output = build_single_file_to_string(input_path)?;
    let js = deka_fmt::format_js(&output.js)?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }

    fs::write(output_path, js)
        .map_err(|err| format!("failed to write {}: {}", output_path.display(), err))?;

    let import_map_path = resolve_import_map_path(output_path);
    let import_map = emit_import_map_json(&output.meta, output_path);
    fs::write(&import_map_path, import_map)
        .map_err(|err| format!("failed to write {}: {}", import_map_path.display(), err))?;

    Ok(())
}

fn build_single_file_bundle_to_path(
    input_path: &Path,
    output_path: &Path,
    minify: bool,
) -> Result<(), String> {
    let output = build_single_file_to_string(input_path)?;
    let entry_js = output.js.clone();
    let entry_path = fs::canonicalize(input_path)
        .map_err(|err| format!("failed to resolve {}: {}", input_path.display(), err))?;
    let provider = Arc::new(PhpxProvider::new(entry_path.clone(), entry_js));
    let bundle = bundle_virtual_entry(
        &entry_path,
        BuildOptions {
            project_root: output.project_root,
            minify,
            iife: false,
        },
        provider,
    )?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }

    let bundle = if minify {
        bundle
    } else {
        deka_fmt::format_js(&bundle)?
    };

    fs::write(output_path, bundle)
        .map_err(|err| format!("failed to write {}: {}", output_path.display(), err))?;

    Ok(())
}

fn build_single_file_to_string(
    input_path: &Path,
) -> Result<JsBuildOutput, String> {
    let input = input_path
        .to_str()
        .ok_or_else(|| format!("invalid utf-8 path: {}", input_path.display()))?;

    let source = fs::read_to_string(input_path)
        .map_err(|err| format!("failed to read {}: {}", input_path.display(), err))?;
    let meta = deka_compile::parse_source_module_meta(&source);

    // Validate the source before checking project layout so that syntax/type
    // errors are surfaced immediately instead of being blocked by a missing
    // deka.lock or php_modules/ directory (dekaruntime/deka#117).
    let js = compile_js_or_report(&source, input)?;

    let project_root = resolve_project_root(input_path)?;
    ensure_project_layout(&project_root, None, &meta)?;

    Ok(JsBuildOutput {
        js,
        meta,
        project_root,
    })
}

struct PhpxProvider {
    entry_path: PathBuf,
    entry_source: String,
}

impl PhpxProvider {
    fn new(entry_path: PathBuf, entry_source: String) -> Self {
        Self {
            entry_path,
            entry_source,
        }
    }
}

impl VirtualSource for PhpxProvider {
    fn load_virtual(&self, path: &Path) -> Result<Option<String>, String> {
        if path == self.entry_path {
            return Ok(Some(self.entry_source.clone()));
        }

        if !is_deka_source_path(path) {
            return Ok(None);
        }

        let input = path
            .to_str()
            .ok_or_else(|| format!("invalid utf-8 path: {}", path.display()))?;
        let source =
            fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", input, err))?;
        let js = compile_js_or_report(&source, input)?;
        Ok(Some(js))
    }
}

fn is_deka_source_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ds") | Some("dsx")
    )
}

