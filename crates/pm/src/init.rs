use deka_cli_core::{CommandSpec, Context, Registry};
use std::path::Path;
use stdio::{error as stdio_error, log, raw};

const COMMAND: CommandSpec = CommandSpec {
    owner: "pm",
    name: "init",
    category: "project",
    summary: "initialize a new app project",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

pub fn cmd(context: &Context) {
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            stdio_error(
                "init",
                &format!("failed to resolve current directory: {}", err),
            );
            return;
        }
    };

    let dir_arg = context.args.positionals.first().map(String::as_str);
    let target = if let Some(dir) = dir_arg {
        cwd.join(dir)
    } else {
        cwd
    };

    raw(&stdio::ascii("deka"));
    raw("");

    if let Err(err) = std::fs::create_dir_all(&target) {
        stdio_error(
            "init",
            &format!("failed to create {}: {}", target.display(), err),
        );
        return;
    }

    match write_scaffold(&target) {
        Ok(touched) if touched.is_empty() => {
            raw("[init] project is already initialized");
        }
        Ok(_) => print_next_steps(dir_arg),
        Err(err) => stdio_error("init", &err),
    }
}

/// The DekaScript scaffold lives as real files under `scaffold/`, embedded
/// with `include_str!` (deka#1000) — readable, diffable, and checked by
/// whatever corpus the compiler already typechecks in CI, instead of a
/// single-line escaped Rust string literal nobody can review.
///
/// Tree matches RFD 24 exactly: `public/`, `app/{layout.dsx,page.dsx}`,
/// `index.html`, `deka.json`. No `src/` (deka#1044); the 404 is a static
/// `public/404.html`, not a rendered route (deka#1045).
fn write_scaffold(target: &Path) -> Result<Vec<String>, String> {
    let mut touched: Vec<String> = Vec::new();
    let name = project_name_from_dir(target);

    ensure_file(
        &target.join("deka.json"),
        default_deka_json(&name),
        &mut touched,
    )?;
    ensure_file(
        &target.join("deka.lock"),
        include_str!("../scaffold/deka.lock").to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join(".gitignore"),
        include_str!("../scaffold/.gitignore").to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("index.html"),
        include_str!("../scaffold/index.html").to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("app/layout.dsx"),
        include_str!("../scaffold/app/layout.dsx").to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("app/page.dsx"),
        include_str!("../scaffold/app/page.dsx").to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("app/Counter.dsx"),
        include_str!("../scaffold/app/Counter.dsx").to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("public/style.css"),
        include_str!("../scaffold/public/style.css").to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("public/404.html"),
        include_str!("../scaffold/public/404.html").to_string(),
        &mut touched,
    )?;
    Ok(touched)
}

fn print_next_steps(dir_arg: Option<&str>) {
    raw("[init] DekaScript app ready");
    match dir_arg {
        Some(dir) if dir != "." && !dir.is_empty() => {
            raw(&format!("  cd {dir}"));
            raw("  deka serve");
        }
        _ => {
            raw("  deka serve");
            raw("  deka dev");
        }
    }
}

fn ensure_file(path: &Path, content: String, touched: &mut Vec<String>) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }
    std::fs::write(path, content.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    let display = path_display(path);
    log("create", &display);
    touched.push(display);
    Ok(())
}

fn project_name_from_dir(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("app")
        .to_string()
}

fn path_display(path: &Path) -> String {
    match std::env::current_dir() {
        Ok(cwd) => match path.strip_prefix(&cwd) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => path.display().to_string(),
        },
        Err(_) => path.display().to_string(),
    }
}

/// The scaffold declares RFD-53 phase-aware `permissions` (deka#757) rather
/// than the legacy `security` shape: `dev` grants exactly what a hello-world
/// app-router project needs to read and serve itself locally (project files,
/// its own build cache, wasm) so `deka dev` / `deka serve --dev` work with
/// zero manual manifest editing (deka#973); `prod` is left empty, i.e.
/// fully denied — production default-deny is unchanged and the emptiness is
/// visible right here rather than hidden behind a manifest the user has no
/// reason to open.
///
/// Templated (not `include_str!`) because it carries the one substitution
/// the scaffold needs: the project name from the target directory.
fn default_deka_json(name: &str) -> String {
    include_str!("../scaffold/deka.json").replace("__APP_NAME__", name)
}

#[cfg(test)]
mod tests {
    use super::default_deka_json;

    #[test]
    fn default_scaffold_is_a_live_ds_app() {
        let json: serde_json::Value = serde_json::from_str(&default_deka_json("demo")).unwrap();
        assert_eq!(json["type"], "serve");
        assert_eq!(json["serve"]["mode"], "ds");
        assert!(json["serve"].get("entry").is_none());
        assert_eq!(json["tasks"]["dev"], "deka serve --dev");

        let index = include_str!("../scaffold/index.html");
        assert!(!index.contains("<!--deka-app-->"));
        assert!(!index.contains("<!--deka-head-->"));
        assert!(!index.contains("<!--deka-scripts-->"));
        assert!(!index.contains("<script"));
        assert!(index.contains("<div id=\"app\"></div>"));
        assert!(index.contains("href=\"/style.css\""));
        assert!(!index.contains("<h1>Deka App</h1>"));

        let page = include_str!("../scaffold/app/page.dsx");
        assert!(page.contains("export fn Page()"));
        assert!(page.contains("<h1>Deka App</h1>"));
        assert!(page.contains("client:load"));
        assert!(page.contains("fn greeting("));
        assert!(page.contains("./Counter.dsx"));
        assert!(!page.contains("src/"));

        let counter = include_str!("../scaffold/app/Counter.dsx");
        assert!(counter.contains("useState"));
        assert!(counter.contains("export fn Counter()"));

        let not_found = include_str!("../scaffold/public/404.html");
        assert!(not_found.contains("Not found"));

        let style = include_str!("../scaffold/public/style.css");
        assert!(style.lines().count() > 1, "style.css must not be one line");

        let gitignore = include_str!("../scaffold/.gitignore");
        assert!(gitignore.contains("ds_modules/"));
        assert!(gitignore.contains("dist/"));
        assert!(!gitignore.contains(".deka.json-backup-*"));
        assert!(!gitignore.contains(".deka.lock-backup-*"));
    }

    /// deka#973: the scaffold must declare RFD-53 phase-aware permissions
    /// (not the legacy `security` shape), granting dev exactly what a
    /// hello-world app-router project needs while leaving prod fully denied
    /// — so a fresh `deka init` needs zero manual manifest editing to serve
    /// in dev, and production default-deny is untouched.
    #[test]
    fn default_scaffold_declares_phase_aware_dev_permissions_and_denies_prod() {
        use permissions::permissions::{Capabilities, ExecutionPhase, FsGrant, parse_permissions};

        let json: serde_json::Value = serde_json::from_str(&default_deka_json("demo")).unwrap();
        assert!(
            json.get("security").is_none(),
            "scaffold must not use the legacy security shape: {json}"
        );

        let outcome = parse_permissions(&json);
        assert!(!outcome.has_errors(), "{:?}", outcome.diagnostics);
        let permissions = outcome
            .permissions
            .expect("scaffold must declare phase-aware permissions.dev/prod");

        assert_eq!(permissions.dev.caps.read, FsGrant::WorkingDir);
        assert!(matches!(permissions.dev.caps.write, FsGrant::Paths(_)));
        if let FsGrant::Paths(paths) = &permissions.dev.caps.write {
            assert!(paths.iter().any(|p| p == ".cache"));
            assert!(paths.iter().any(|p| p == "ds_modules/.cache"));
        }
        assert!(permissions.dev.caps.wasm);

        assert!(
            permissions.prod.caps.is_fully_denied(),
            "prod must stay default-deny: {:?}",
            permissions.prod.caps
        );
        assert_eq!(permissions.prod.caps, Capabilities::default());

        // Resolve() must produce a working dev grant and a fully-denied prod
        // grant for the exact working directory a served project would use.
        let working_dir = std::path::Path::new("/tmp/does-not-need-to-exist");
        let dev_policy = permissions.resolve(ExecutionPhase::DevRequest, working_dir);
        assert_ne!(
            dev_policy.allow.read,
            ::security::security_policy::RuleList::None
        );
        let prod_policy = permissions.resolve(ExecutionPhase::ProdRequest, working_dir);
        assert_eq!(
            prod_policy.allow.read,
            ::security::security_policy::RuleList::None
        );
    }
}
