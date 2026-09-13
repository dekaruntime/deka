use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn resolve_project_root(input_path: &Path) -> Result<PathBuf, String> {
    let start = project_root_search_start(input_path);

    let mut nearest_manifest_root = None;
    for dir in start.ancestors() {
        if dir.join("deka.json").is_file() {
            let dir = dir.to_path_buf();
            if dir.join("deka.lock").is_file() {
                return canonical_project_root(dir);
            }
            if nearest_manifest_root.is_none() {
                nearest_manifest_root = Some(dir);
            }
        }
    }

    if let Some(root) = nearest_manifest_root {
        return canonical_project_root(root);
    }

    Err(format!(
        "deka build requires a deka.json project root (searched from {})",
        input_path.display()
    ))
}

/// Resolve the root once at the command boundary. Downstream build phases
/// compare dsc-emitted source paths with scanned project paths, so preserving
/// a caller spelling such as `.` would make one project appear to have two
/// different roots.
fn canonical_project_root(root: PathBuf) -> Result<PathBuf, String> {
    fs::canonicalize(&root)
        .map_err(|err| format!("failed to resolve project root {}: {err}", root.display()))
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

pub(super) fn ensure_project_layout(
    project_root: &Path,
    module_root: Option<&Path>,
    imports: &[String],
) -> Result<(), String> {
    deka_modules::project_gate::validate_project(
        project_root,
        imports,
        &deka_modules::project_gate::GateOptions {
            module_root: module_root.map(|p| p.to_path_buf()),
            require_lockfile: true,
            context: "deka build",
        },
    )
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

pub(super) fn ensure_web_project_layout(project_root: &Path) -> Result<(), String> {
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

    if project_root.join("index.html").is_file()
        && project_root.join("public").join("index.html").is_file()
    {
        return Err("public/index.html collides with the root index.html document".to_string());
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

pub(super) fn resolve_web_entry(project_root: &Path) -> Result<PathBuf, String> {
    let json = load_deka_json(project_root)?;

    if runtime_core::dist::is_source_app_router_project(project_root) {
        let page = project_root.join("app").join("page.dsx");
        let page = if page.is_file() {
            page
        } else {
            project_root.join("app").join("page.ds")
        };
        return Ok(page);
    }

    let entry = json
        .get("serve")
        .and_then(|v| v.get("entry"))
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "web build requires index.html + app/page.dsx, or deka.json serve.entry, in {}",
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

pub(super) fn is_deka_source_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ds") | Some("dsx")
    )
}
