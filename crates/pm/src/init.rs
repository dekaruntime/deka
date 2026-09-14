use deka_cli_core::{CommandSpec, Context, Registry};
use std::path::Path;
use stdio::{error as stdio_error, raw};

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

fn write_scaffold(target: &Path) -> Result<Vec<String>, String> {
    let mut touched: Vec<String> = Vec::new();
    ensure_file(
        &target.join("deka.json"),
        default_deka_json(project_name_from_dir(target).as_str()),
        &mut touched,
    )?;
    ensure_file(
        &target.join("deka.lock"),
        default_deka_lock_json().to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join(".gitignore"),
        default_gitignore().to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("index.html"),
        default_index_html().to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("app/layout.dsx"),
        default_app_layout_dsx().to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("app/page.dsx"),
        default_app_page_dsx().to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("app/not-found.dsx"),
        default_not_found_dsx().to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("src/ui/Counter.dsx"),
        default_counter_dsx().to_string(),
        &mut touched,
    )?;
    ensure_file(
        &target.join("public/style.css"),
        default_public_style_css().to_string(),
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
    touched.push(path_display(path));
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
fn default_deka_json(name: &str) -> String {
    format!(
        "{{\n  \"name\": \"{name}\",\n  \"type\": \"serve\",\n  \"serve\": {{ \"mode\": \"ds\" }},\n  \"tasks\": {{ \"dev\": \"deka serve --dev\" }},\n  \"permissions\": {{\n    \"dev\": {{\n      \"read\": true,\n      \"write\": [\".cache\", \"ds_modules/.cache\"],\n      \"wasm\": true\n    }},\n    \"prod\": {{}}\n  }}\n}}\n"
    )
}

fn default_deka_lock_json() -> &'static str {
    "{\n  \"lockfileVersion\": 1,\n  \"packages\": {}\n}\n"
}

fn default_gitignore() -> &'static str {
    "ds_modules/\ndist/\n.cache/\n.deka.json-backup-*\n.deka.lock-backup-*\n"
}

fn default_index_html() -> &'static str {
    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>Deka</title>\n    <link rel=\"stylesheet\" href=\"/style.css\" />\n    <!--deka-head-->\n  </head>\n  <body>\n    <div id=\"app\"><!--deka-app--></div>\n    <!--deka-scripts-->\n  </body>\n</html>\n"
}

fn default_app_page_dsx() -> &'static str {
    "import { Counter } from \"../src/ui/Counter.dsx\"\n\nfn greeting(name: string) string {\n  return \"Hello, \" + name + \".\"\n}\n\nexport fn Page() ReactNode {\n  return <section>\n      <h1>Deka App</h1>\n      <p>{greeting(\"World\")}</p>\n      <Counter client:load />\n    </section>\n}\n"
}

fn default_app_layout_dsx() -> &'static str {
    "interface LayoutProps {\n  children: ReactNode\n}\n\nexport fn Layout(props: LayoutProps) ReactNode {\n  return <main>{props.children}</main>\n}\n"
}

fn default_not_found_dsx() -> &'static str {
    "export fn Page() ReactNode {\n  return <section><h1>Not found</h1></section>\n}\n"
}

fn default_counter_dsx() -> &'static str {
    "export fn Counter() ReactNode {\n  const pair = useState(0)\n  const n = pair[0]\n  const setN = pair[1]\n  return <button type=\"button\" id=\"counter\" onClick={fn() void {\n      setN(n + 1)\n    }}>{n}</button>\n}\n"
}

fn default_public_style_css() -> &'static str {
    "body { font-family: system-ui, sans-serif; margin: 2rem; }\n"
}

#[cfg(test)]
mod tests {
    use super::{
        default_app_page_dsx, default_counter_dsx, default_deka_json, default_gitignore,
        default_index_html,
    };

    #[test]
    fn default_scaffold_is_a_live_ds_app() {
        let json: serde_json::Value = serde_json::from_str(&default_deka_json("demo")).unwrap();
        assert_eq!(json["type"], "serve");
        assert_eq!(json["serve"]["mode"], "ds");
        assert!(json["serve"].get("entry").is_none());
        assert_eq!(json["tasks"]["dev"], "deka serve --dev");

        let index = default_index_html();
        assert!(index.contains("<!--deka-app-->"));
        assert!(index.contains("<!--deka-head-->"));
        assert!(index.contains("<!--deka-scripts-->"));
        assert!(index.contains("href=\"/style.css\""));
        assert!(!index.contains("<h1>Deka App</h1>"));

        let page = default_app_page_dsx();
        assert!(page.contains("export fn Page()"));
        assert!(page.contains("<h1>Deka App</h1>"));
        assert!(page.contains("client:load"));
        assert!(page.contains("fn greeting("));

        let counter = default_counter_dsx();
        assert!(counter.contains("useState"));
        assert!(counter.contains("export fn Counter()"));

        let gitignore = default_gitignore();
        assert!(gitignore.contains("ds_modules/"));
        assert!(gitignore.contains("dist/"));
        assert!(gitignore.contains(".deka.json-backup-*"));
        assert!(gitignore.contains(".deka.lock-backup-*"));
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
