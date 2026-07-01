use bundler::{BundleOptions, VirtualSource, bundle_virtual_entry};
use core::{CommandSpec, Context, ParamSpec, Registry};
use phpx_js::{SourceModuleMeta, compile_phpx_source_to_js, parse_source_module_meta};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const COMMAND: CommandSpec = CommandSpec {
    name: "build",
    category: "project",
    summary: "build a PHPX file into a JavaScript module (JS runtime semantics)",
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
        if first.ends_with(".phpx") {
            return run_single_file_build(context, first);
        }
    }

    run_web_project_build(context)
}

fn run_single_file_build(context: &Context, input: &str) -> Result<(), String> {
    if !input.ends_with(".phpx") {
        return Err(format!(
            "build currently supports .phpx input only; got '{}'",
            input
        ));
    }

    let input_path = PathBuf::from(input);
    let output_path = resolve_output_path(output_arg(context), &input_path)?;
    if bundle_enabled(context) {
        build_single_file_bundle_to_path(&input_path, &output_path, minify_enabled(context))?;
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

    let modules_dir = project_root.join("php_modules");
    if modules_dir.is_dir() {
        copy_dir_recursive(&modules_dir, &dist_server.join("php_modules"))?;
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

fn emit_import_map_json(meta: &SourceModuleMeta, output_path: &Path) -> String {
    let mut imports = default_import_map();

    for decl in &meta.imports {
        let spec = decl.from.trim();
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
            "/php_modules/component/".to_string(),
        ),
        ("deka/".to_string(), "/php_modules/deka/".to_string()),
        (
            "encoding/".to_string(),
            "/php_modules/encoding/".to_string(),
        ),
        ("db/".to_string(), "/php_modules/db/".to_string()),
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
    rel.push_str("php_modules/");
    rel.push_str(spec.trim_start_matches('/'));

    if !rel.ends_with(".js") && !rel.ends_with('/') {
        rel.push_str(".js");
    }

    rel
}

fn resolve_project_root(input_path: &Path) -> Result<PathBuf, String> {
    let start = if input_path.is_dir() {
        input_path.to_path_buf()
    } else {
        input_path.parent().unwrap_or(Path::new(".")).to_path_buf()
    };

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

fn ensure_project_layout(project_root: &Path, meta: &SourceModuleMeta) -> Result<(), String> {
    // PHPX_MODULE_ROOT bypass (#220): when set, the tenant relies on the runtime stdlib at
    // that root and we trust the runtime-provided modules without requiring a local
    // deka.lock or php_modules/. Tenant-local packages would still need a lockfile, but
    // stdlib-only tenants (id.tana.gg) deploy without ceremony.
    if std::env::var_os("PHPX_MODULE_ROOT").is_some() {
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

    let modules_dir = project_root.join("php_modules");
    if !modules_dir.is_dir() {
        return Err(format!(
            "deka build requires php_modules/ at project root when using stdlib imports ({}). Run `deka install`.",
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

fn collect_stdlib_imports(meta: &SourceModuleMeta) -> Vec<String> {
    let mut seen = BTreeSet::new();
    for decl in &meta.imports {
        let spec = decl.from.trim();
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
                | "cookies"
                | "auth"
                | "db"
        )
}

fn resolve_module_file(modules_dir: &Path, spec: &str) -> Option<PathBuf> {
    let mut candidates = vec![
        modules_dir.join(format!("{}.phpx", spec)),
        modules_dir.join(format!("{}.php", spec)),
        modules_dir.join(spec).join("index.phpx"),
        modules_dir.join(spec).join("index.php"),
    ];

    // For prefixed stdlib specifiers (e.g. encoding/json) also check the scoped
    // @deka layout — stdlib packages installed via `deka install` live there.
    if spec.contains('/')
        && !spec.starts_with('@')
        && !spec.starts_with("./")
        && !spec.starts_with("../")
    {
        let scoped = format!("@deka/{}", spec);
        candidates.push(modules_dir.join(format!("{}.phpx", scoped)));
        candidates.push(modules_dir.join(format!("{}.php", scoped)));
        candidates.push(modules_dir.join(&scoped).join("index.phpx"));
        candidates.push(modules_dir.join(&scoped).join("index.php"));
    }

    if spec.ends_with(".phpx") || spec.ends_with(".php") {
        candidates.insert(0, modules_dir.join(spec));
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

    let deka_php = project_root.join("php_modules").join("deka.php");
    if !deka_php.is_file() {
        return Err(format!(
            "missing required runtime prelude: {} (run `deka init`)",
            deka_php.display()
        ));
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
                "web build requires deka.json serve.entry (example: \"app/main.phpx\") in {}",
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

    if entry_path.extension().and_then(|e| e.to_str()) != Some("phpx") {
        return Err(format!(
            "serve.entry must be a .phpx file: {}",
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

fn extract_template_html(source: &str) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let (start, end) = frontmatter_range(&lines)?;
    if end + 1 >= lines.len() {
        return None;
    }
    let template = lines[end + 1..].join("\n");
    let bindings = parse_frontmatter_bindings(&lines[start..end]);
    let rendered = apply_frontmatter_bindings(&template, &bindings);
    let trimmed = rendered.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn frontmatter_range(lines: &[&str]) -> Option<(usize, usize)> {
    let mut first = None;
    let mut second = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.trim() == "---" {
            if first.is_none() {
                first = Some(idx);
            } else {
                second = Some(idx);
                break;
            }
        }
    }
    match (first, second) {
        (Some(a), Some(b)) if b > a => Some((a + 1, b)),
        _ => None,
    }
}

fn unquote(input: &str) -> Option<&str> {
    let s = input.trim();
    if s.len() < 2 {
        return None;
    }
    let first = s.as_bytes()[0] as char;
    let last = s.as_bytes()[s.len() - 1] as char;
    if (first == '\'' && last == '\'') || (first == '"' && last == '"') {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

fn parse_frontmatter_bindings(lines: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in lines {
        let trimmed = line.trim().trim_end_matches(';').trim();
        if !trimmed.starts_with('$') {
            continue;
        }
        let Some((lhs, rhs)) = trimmed.split_once('=') else {
            continue;
        };
        let key = lhs.trim().trim_start_matches('$').trim();
        if key.is_empty() {
            continue;
        }
        let value = rhs.trim();
        if let Some(unquoted) = unquote(value) {
            out.insert(key.to_string(), unquoted.to_string());
        }
    }
    out
}

fn apply_frontmatter_bindings(template: &str, bindings: &BTreeMap<String, String>) -> String {
    let mut out = template.to_string();
    for (key, value) in bindings {
        let token = format!("{{${}}}", key);
        out = out.replace(&token, value);
    }
    out
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
    meta: SourceModuleMeta,
    project_root: PathBuf,
}

fn build_single_file_to_path(input_path: &Path, output_path: &Path) -> Result<(), String> {
    let output = build_single_file_to_string(input_path)?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }

    fs::write(output_path, output.js)
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
    let prelude = phpx_js::build_stdlib_prelude(&output.project_root)?;
    let entry_js = format!("{prelude}\n{}", output.js);
    let entry_path = fs::canonicalize(input_path)
        .map_err(|err| format!("failed to resolve {}: {}", input_path.display(), err))?;
    let provider = Arc::new(PhpxBundleProvider::new(entry_path.clone(), entry_js));
    let bundle = bundle_virtual_entry(
        &entry_path,
        BundleOptions {
            project_root: output.project_root,
            minify,
            iife: false,
            stdlib_path: None,
        },
        provider,
    )?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }

    fs::write(output_path, bundle)
        .map_err(|err| format!("failed to write {}: {}", output_path.display(), err))?;

    Ok(())
}

fn build_single_file_to_string(input_path: &Path) -> Result<JsBuildOutput, String> {
    let input = input_path
        .to_str()
        .ok_or_else(|| format!("invalid utf-8 path: {}", input_path.display()))?;

    let source = fs::read_to_string(input_path)
        .map_err(|err| format!("failed to read {}: {}", input_path.display(), err))?;
    let meta = parse_source_module_meta(&source);

    let project_root = resolve_project_root(input_path)?;
    ensure_project_layout(&project_root, &meta)?;

    let js = compile_phpx_source_to_js(&source, input, meta.clone())?;

    Ok(JsBuildOutput {
        js,
        meta,
        project_root,
    })
}

struct PhpxBundleProvider {
    entry_path: PathBuf,
    entry_source: String,
}

impl PhpxBundleProvider {
    fn new(entry_path: PathBuf, entry_source: String) -> Self {
        Self {
            entry_path,
            entry_source,
        }
    }
}

impl VirtualSource for PhpxBundleProvider {
    fn load_virtual(&self, path: &Path) -> Result<Option<String>, String> {
        if path == self.entry_path {
            return Ok(Some(self.entry_source.clone()));
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("phpx") {
            return Ok(None);
        }

        let input = path
            .to_str()
            .ok_or_else(|| format!("invalid utf-8 path: {}", path.display()))?;
        let source =
            fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", input, err))?;
        let meta = parse_source_module_meta(&source);
        let js = compile_phpx_source_to_js(&source, input, meta)?;
        Ok(Some(js))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bumpalo::Bump;
    use php_rs::parser::lexer::Lexer;
    use php_rs::parser::parser::{Parser, ParserMode};
    use phpx_js::{emit_js_from_ast, emit_js_scaffold_with_reason};

    #[test]
    fn default_outdir_is_dist_js() {
        let path = resolve_output_path(None, Path::new("src/home.phpx")).expect("path");
        assert_eq!(path, PathBuf::from("dist/home.js"));
    }

    #[test]
    fn explicit_out_is_used() {
        let path =
            resolve_output_path(Some("build/out.js".to_string()), Path::new("src/home.phpx"))
                .expect("path");
        assert_eq!(path, PathBuf::from("build/out.js"));
    }

    #[test]
    fn parses_import_and_export_meta() {
        let source = r#"
---
import { jsx, jsxs as pack } from 'component/core'
export function Card($props: object) {
  return <div>{$props.title}</div>
}
export { Card as Panel }
---
<div>ok</div>
"#;
        let meta = parse_source_module_meta(source);
        assert_eq!(meta.imports.len(), 1);
        assert!(meta.exported_functions.contains("Card"));
        assert_eq!(meta.export_specs.len(), 1);
        assert_eq!(meta.export_specs[0].local, "Card");
        assert_eq!(meta.export_specs[0].imported, "Panel");
    }

    #[test]
    fn emitted_js_contains_runtime_hook_for_fallback() {
        let js = emit_js_scaffold_with_reason("echo 'hi'", "main.phpx", "x");
        assert!(js.contains("runPhpx"));
        assert!(js.contains("runtime.executePhpx"));
        assert!(js.contains("phpxTargetSemantics = \"js\""));
    }

    #[test]
    fn rewrites_deka_i_import_to_runtime_helper() {
        let source = r#"
function Hello($props: object) {
  return <span>Hello {$props.name}</span>
}
$view = <div><Hello name="world" /></div>
"#;
        let meta_source = "---
import { i } from 'deka/i'
---
<div />
";
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let meta = parse_source_module_meta(meta_source);
        let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("subset emit");
        assert!(!js.contains("from 'deka/i'"));
        assert!(js.contains("const __phpxTypeRegistry = {}"));
        assert!(js.contains("const i = __deka_i;"));
        assert!(js.contains("safeParse"));
    }

    #[test]
    fn captures_struct_schema_for_deka_i_registry() {
        let source = r#"
struct User {
  $name: string
  $age: Option<int>
}
function Hello($props: object) {
  return <span>Hello {$props.name}</span>
}
$view = <div><Hello name="world" /></div>
"#;
        let meta_source = "---
import { i } from 'deka/i'
---
<div />
";
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let meta = parse_source_module_meta(meta_source);
        let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("subset emit");
        assert!(js.contains("__phpxTypeRegistry[\"User\"]"));
        assert!(js.contains("\"name\": { schema: { kind: 'string' }, optional: false }"));
        assert!(js.contains("\"age\": { schema:"));
        assert!(js.contains("kind: 'number'"));
    }

    #[test]
    fn emits_subset_ast_for_jsx_component() {
        let source = r#"
function Hello($props: object) {
  return <span style="color:red">Hello {$props.name}</span>
}
$view = <div><Hello name="world" /></div>
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("phpxBuildMode = \"subset-ast\""));
        assert!(js.contains("phpxTargetSemantics = \"js\""));
        assert!(js.contains("import { jsx, jsxs } from 'component/core'"));
        assert!(js.contains("function Hello(props)"));
        assert!(js.contains("let view = jsx"));
    }

    #[test]
    fn emits_if_and_unary() {
        let source = r#"
function Flag($props: object) {
  if (!$props.on) {
    return "off"
  }
  return "on"
}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("if ((!props.on))"));
        assert!(js.contains("return \"off\""));
    }
    #[test]
    fn emits_match_and_array_dim_fetch() {
        let source = r#"
function Pick($props: object) {
  $name = match ($props.role) {
    "owner" => "sam",
    "member", "user" => "guest",
    default => "anon",
  }
  return [$name, $props.items[0]][0]
}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("props.items[0]"));
        assert!(js.contains("props.role === \"owner\""));
        assert!(js.contains("props.role === \"member\""));
        assert!(js.contains("props.role === \"user\""));
    }

    #[test]
    fn emits_named_arg_call_as_object_payload() {
        let source = r#"
function greet($props: object) {
  return $props.name
}
$value = greet(name: "Sami")
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("greet({\"name\": \"Sami\"})"));
    }

    #[test]
    fn emits_keyed_array_as_object_literal() {
        let source = r#"
$data = ["name" => "Sami", "role" => "owner"]
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("{\"name\": \"Sami\", \"role\": \"owner\"}"));
    }

    #[test]
    fn emits_method_and_nullsafe_access() {
        let source = r#"
$value = $user->getName()
$title = $user?->profile?->title
$nick = $user?->getNick()
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        // Variables not previously declared emit as globalThis.X
        assert!(js.contains("globalThis.user.getName()"));
        assert!(js.contains("(globalThis.user)?.profile"));
        assert!(js.contains("(globalThis.user)?.getNick()"));
    }

    #[test]
    fn emits_while_for_foreach_statements() {
        let source = r#"
$i = 0
while ($i < 2) {
  $i = $i + 1
}
for ($j = 0; $j < 2; $j = $j + 1) {
  $i = $i + $j
}
foreach ($items as $idx => $item) {
  $i = $i + $idx
}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        // i and j are locally declared by prior assignment; items is undeclared (globalThis)
        assert!(js.contains("while ((i < 2))"));
        assert!(js.contains("for (let j = 0; (j < 2); (j = (j + 1)))"));
        assert!(js.contains("for (const [idx , item] of Object.entries(globalThis.items))"));
    }

    #[test]
    fn emits_break_continue_and_cast_helpers() {
        let source = r#"
for ($i = 0; $i < 10; $i = $i + 1) {
  if ($i == 2) {
    continue
  }
  if ($i == 8) {
    break
  }
}
$ok = isset($user.name)
$missing = empty($user.name)
$num = (int)"42"
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("continue;"));
        assert!(js.contains("if ((i === 2))"));
        assert!(js.contains("if ((i === 8))"));
        assert!(js.contains("break;"));
        assert!(js.contains("!== undefined"));
        assert!(js.contains("Number.parseInt(\"42\", 10)"));
    }

    #[test]
    fn emits_switch_and_dowhile_statements() {
        let source = r#"
$i = 0
do {
  $i = $i + 1
} while ($i < 2)
switch ($i) {
  case 1:
    $i = 10
    break
  default:
    $i = 20
}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("do {"));
        assert!(js.contains("while ((i < 2));"));
        assert!(js.contains("switch (i)"));
        assert!(js.contains("case 1:"));
        assert!(js.contains("default:"));
    }

    #[test]
    fn emits_arrow_function_expression() {
        let source = r#"
$fn = fn ($x: int) => $x + 1
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("let fn = (x) => (x + 1);"));
    }

    #[test]
    fn emits_print_and_inline_html() {
        let source = r#"
print("hi")
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("console.log(\"hi\")"));
    }

    #[test]
    fn emits_closure_expression() {
        let source = r#"
$fn = function ($x: int) {
  return $x + 1
}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("function(x)"));
        assert!(js.contains("return (x + 1);"));
    }

    #[test]
    fn emits_assign_ops_and_inc_dec() {
        let source = r#"
$counter = 1;
$counter += 2;
$counter--;
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("let counter = 1;"));
        assert!(js.contains("counter += 2"));
        assert!(js.contains("counter--)"));
    }

    #[test]
    fn emits_throw_statement() {
        let source = r#"
throw "boom";
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("throw \"boom\";"));
    }

    #[test]
    fn emits_static_call_and_class_const_fetch() {
        let source = r#"
$value = Option::Some(1);
$status = Http::OK;
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("Option.Some(1)"));
        assert!(js.contains("Http.OK"));
    }

    #[test]
    fn emits_struct_literal_as_object() {
        let source = r#"
$user = User { $id: 1, $name: "sam" };
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("\"__struct\": \"User\""));
        assert!(js.contains("\"id\": 1"));
        assert!(js.contains("\"name\": \"sam\""));
    }

    #[test]
    fn emits_new_expression() {
        let source = r#"
$point = new Point(1, 2);
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("new Point(1, 2)") || js.contains("new globalThis.Point(1, 2)"));
    }

    #[test]
    fn emits_magic_constants() {
        let source = r#"
$line = __LINE__;
$file = __FILE__;
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("let line = 0;"));
        assert!(js.contains("let file = \"\";"));
    }

    #[test]
    fn emits_assign_ref_as_assignment() {
        let source = r#"
$a = 1;
$b = &$a;
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("let a = 1;"));
        assert!(js.contains("let b = a;"));
    }

    #[test]
    fn emits_include_with_runtime_stub() {
        let source = r#"
$value = include "./part.phpx";
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("function __phpx_include(path, kind)"));
        assert!(js.contains("__phpx_include(\"./part.phpx\", \"include\")"));
    }

    #[test]
    fn emits_const_statement() {
        let source = r#"
const ANSWER = 42;
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("const ANSWER = 42;"));
    }

    #[test]
    fn emits_try_catch_finally_statement() {
        let source = r#"
try {
  throw "boom";
} catch (Exception $e) {
  $msg = $e;
} finally {
  $done = true;
}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("try {"));
        assert!(js.contains("catch (e) {"));
        assert!(js.contains("finally {"));
    }

    #[test]
    fn emits_global_static_unset_statements() {
        let source = r#"
function demo($x: int): int {
  global $shared;
  static $count = 1;
  unset($shared);
  return $x;
}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        // global statement now imports from globalThis; static is a local let
        assert!(js.contains("globalThis.shared"));
        assert!(js.contains("let count = 1;"));
        assert!(js.contains("shared = undefined;"));
    }
    #[test]
    fn emits_label_goto_declare_and_halt_compiler_statements() {
        let source = r#"
start:
declare(ticks=1) {
  $x = 1;
}
goto start;
__halt_compiler();
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("label start ignored"));
        assert!(js.contains("goto start is not supported"));
        assert!(js.contains("let x = 1;"));
        assert!(js.contains("__halt_compiler ignored"));
    }

    #[test]
    fn emits_await_eval_and_clone_expressions() {
        let source = r#"
$one = await $promise;
$two = eval("40 + 2");
$three = clone $obj;
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        // Undeclared variables get globalThis prefix
        assert!(js.contains("await globalThis.promise"));
        assert!(js.contains("eval(\"40 + 2\")"));
        assert!(js.contains("structuredClone(globalThis.obj)"));
    }

    #[test]
    fn emits_die_and_exit_expressions() {
        let source = r#"
$one = die("boom");
$two = exit();
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect("subset emit");
        assert!(js.contains("throw new Error(String(\"boom\"))"));
        assert!(js.contains("throw new Error(\"exit\")"));
    }

    #[test]
    fn reports_specific_error_for_shell_exec_expression() {
        let source = r#"
$out = `echo hi`;
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let err = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect_err("subset emit should fail");
        assert!(err.contains("shell execution is not supported"));
    }

    #[test]
    fn reports_specific_error_for_yield_expression() {
        let source = r#"
function gen() {
  yield 1;
}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let err = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect_err("subset emit should fail");
        assert!(err.contains("yield expressions are not supported"));
    }

    #[test]
    fn reports_specific_error_for_namespace_statement() {
        let source = r#"
namespace Demo;
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let err = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect_err("subset emit should fail");
        assert!(err.contains("namespace declarations are not supported"));
    }

    #[test]
    fn reports_specific_error_for_class_statement() {
        let source = r#"
class User {}
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let err = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
            .expect_err("subset emit should fail");
        assert!(err.contains("class-like declarations are not supported"));
    }

    #[test]
    fn project_root_requires_deka_json() {
        let tmp = tempfile::tempdir().expect("tmp");
        let input = tmp.path().join("src").join("main.phpx");
        std::fs::create_dir_all(input.parent().expect("parent")).expect("mkdir");
        std::fs::write(&input, "<div />").expect("write");

        let err = resolve_project_root(&input).expect_err("should fail without deka.json");
        assert!(err.contains("deka.json"));
    }

    #[test]
    fn ensure_project_layout_requires_lock_file() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("deka.json");
        let meta = SourceModuleMeta::empty();
        let err = ensure_project_layout(tmp.path(), &meta).expect_err("missing lock");
        assert!(err.contains("deka.lock"));
    }

    #[test]
    fn project_root_prefers_outer_lock_bearing_root_over_nested_package_manifest() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("root deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("root deka.lock");

        let package_dir = tmp
            .path()
            .join("php_modules")
            .join("@deka")
            .join("payments");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        std::fs::write(package_dir.join("deka.json"), "{}").expect("package deka.json");
        let input = package_dir.join("index.phpx");
        std::fs::write(&input, "<div />").expect("input");

        let root = resolve_project_root(&input).expect("project root");
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn project_root_falls_back_to_nearest_manifest_when_no_lock_exists() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("root deka.json");

        let package_dir = tmp
            .path()
            .join("php_modules")
            .join("@deka")
            .join("payments");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        std::fs::write(package_dir.join("deka.json"), "{}").expect("package deka.json");
        let input = package_dir.join("index.phpx");
        std::fs::write(&input, "<div />").expect("input");

        let root = resolve_project_root(&input).expect("project root");
        assert_eq!(root, package_dir);
    }

    #[test]
    fn ensure_project_layout_requires_php_modules_for_stdlib_imports() {
        // No stdlib fallback — only project-local php_modules/ is consulted.
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");
        let source = "---\nimport { parse } from 'encoding/json'\n---\n<div />\n";
        let meta = parse_source_module_meta(source);
        let err = ensure_project_layout(tmp.path(), &meta).expect_err("missing php_modules");
        assert!(err.contains("php_modules"));
    }

    #[test]
    fn ensure_project_layout_requires_stdlib_module_files() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");
        std::fs::create_dir_all(tmp.path().join("php_modules")).expect("php_modules");
        let source = "---\nimport { parse } from 'encoding/json'\n---\n<div />\n";
        let meta = parse_source_module_meta(source);
        let err = ensure_project_layout(tmp.path(), &meta).expect_err("missing stdlib module file");
        assert!(err.contains("encoding/json"));
    }

    #[test]
    fn ensure_project_layout_accepts_present_stdlib_module_file() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");
        std::fs::create_dir_all(tmp.path().join("php_modules").join("encoding"))
            .expect("encoding dir");
        std::fs::write(
            tmp.path()
                .join("php_modules")
                .join("encoding")
                .join("json.phpx"),
            "export function parse($v: string): object { return {} }",
        )
        .expect("json.phpx");
        let source = "---\nimport { parse } from 'encoding/json'\n---\n<div />\n";
        let meta = parse_source_module_meta(source);
        ensure_project_layout(tmp.path(), &meta).expect("layout should pass");
    }

    #[test]
    fn build_bundle_emits_single_file() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");

        let php_modules = tmp.path().join("php_modules");
        std::fs::create_dir_all(&php_modules).expect("php_modules dir");
        std::fs::write(php_modules.join("stdlib.json"), "[]").expect("stdlib.json");

        let app_dir = tmp.path().join("app");
        std::fs::create_dir_all(&app_dir).expect("app dir");
        std::fs::write(
            app_dir.join("util.phpx"),
            "export function shout($text: string): string { return $text . \"!\"; }",
        )
        .expect("util.phpx");
        std::fs::write(
            app_dir.join("main.phpx"),
            "import { shout } from './util.phpx'\n\necho shout(\"ok\")\n",
        )
        .expect("main.phpx");

        let output_path = tmp.path().join("bundle.js");
        build_single_file_bundle_to_path(&app_dir.join("main.phpx"), &output_path, false)
            .expect("bundle");
        let output = std::fs::read_to_string(&output_path).expect("bundle output");
        assert!(!output.contains("import {"));
    }

    #[test]
    fn ensure_web_project_layout_requires_deka_php_prelude() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(
            tmp.path().join("deka.json"),
            "{\"type\":\"serve\",\"serve\":{\"entry\":\"app/main.phpx\"}}",
        )
        .expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");
        std::fs::create_dir_all(tmp.path().join("app")).expect("app");
        std::fs::create_dir_all(tmp.path().join("public")).expect("public");
        std::fs::write(
            tmp.path().join("public").join("index.html"),
            "<html></html>",
        )
        .expect("index");

        let err = ensure_web_project_layout(tmp.path()).expect_err("deka.php should be required");
        assert!(err.contains("php_modules/deka.php"));
    }

    #[test]
    fn ensure_web_project_layout_requires_type_serve() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(
            tmp.path().join("deka.json"),
            "{\"type\":\"lib\",\"serve\":{\"entry\":\"app/main.phpx\"}}",
        )
        .expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");
        std::fs::create_dir_all(tmp.path().join("app")).expect("app");
        std::fs::create_dir_all(tmp.path().join("public")).expect("public");
        std::fs::create_dir_all(tmp.path().join("php_modules")).expect("php_modules");
        std::fs::write(
            tmp.path().join("public").join("index.html"),
            "<html></html>",
        )
        .expect("index");
        std::fs::write(tmp.path().join("php_modules").join("deka.php"), "<?php ?>")
            .expect("deka.php");

        let err = ensure_web_project_layout(tmp.path()).expect_err("type should be required");
        assert!(err.contains("type=\"serve\""));
    }

    #[test]
    fn ensure_web_project_layout_accepts_type_serve() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(
            tmp.path().join("deka.json"),
            "{\"type\":\"serve\",\"serve\":{\"entry\":\"app/main.phpx\"}}",
        )
        .expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");
        std::fs::create_dir_all(tmp.path().join("app")).expect("app");
        std::fs::create_dir_all(tmp.path().join("public")).expect("public");
        std::fs::create_dir_all(tmp.path().join("php_modules")).expect("php_modules");
        std::fs::write(
            tmp.path().join("public").join("index.html"),
            "<html></html>",
        )
        .expect("index");
        std::fs::write(tmp.path().join("php_modules").join("deka.php"), "<?php ?>")
            .expect("deka.php");

        ensure_web_project_layout(tmp.path()).expect("web layout should pass");
    }

    #[test]
    fn import_map_path_uses_output_directory() {
        let path = resolve_import_map_path(Path::new("dist/home.js"));
        assert_eq!(path, PathBuf::from("dist/importmap.json"));
    }

    #[test]
    fn import_map_adds_fallback_for_unmapped_bare_specifiers() {
        let source = "---
import { thing } from 'acme/widgets'
---
<div />
";
        let meta = parse_source_module_meta(source);
        let json = emit_import_map_json(&meta, Path::new("dist/home.js"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        let imports = value
            .get("imports")
            .and_then(|v| v.as_object())
            .expect("imports object");

        assert_eq!(
            imports.get("acme/widgets").and_then(|v| v.as_str()),
            Some("php_modules/acme/widgets.js")
        );
        assert_eq!(
            imports.get("component/").and_then(|v| v.as_str()),
            Some("/php_modules/component/")
        );
    }

    #[test]
    fn import_map_skips_relative_and_url_imports() {
        let source = "---
import { x } from './local.js'
import { y } from 'https://cdn.example/x.js'
---
<div />
";
        let meta = parse_source_module_meta(source);
        let json = emit_import_map_json(&meta, Path::new("dist/home.js"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        let imports = value
            .get("imports")
            .and_then(|v| v.as_object())
            .expect("imports object");

        assert!(!imports.contains_key("./local.js"));
        assert!(!imports.contains_key("https://cdn.example/x.js"));
    }
}
