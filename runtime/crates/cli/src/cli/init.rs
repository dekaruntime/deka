use core::{CommandSpec, Context, Registry};
use pm::{InstallPayload, run_install};
use std::path::Path;
use stdio::{error as stdio_error, raw};

const COMMAND: CommandSpec = CommandSpec {
    name: "init",
    category: "project",
    summary: "initialize a new app project",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

const DEFAULT_PHP_PACKAGES: &[&str] = &[
    "@deka/array",
    "@deka/component",
    "@deka/core",
    "@deka/encoding",
    "@deka/fs",
    "@deka/json",
    "@deka/string",
    "@deka/time",
];

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

    let target = if let Some(dir) = context.args.positionals.first() {
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

    let mut touched: Vec<String> = Vec::new();

    if let Err(err) = ensure_file(
        &target.join("deka.json"),
        default_deka_json(project_name_from_dir(&target).as_str()),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }

    if let Err(err) = ensure_file(
        &target.join("deka.lock"),
        default_deka_lock_json(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }

    if let Err(err) = std::fs::create_dir_all(target.join("app")) {
        stdio_error("init", &format!("failed to create app/: {}", err));
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("app").join("main.ds"),
        default_main_ds().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("app").join("page.ds"),
        default_app_page_ds().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("app").join("layout.ds"),
        default_app_layout_ds().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }

    if let Err(err) = std::fs::create_dir_all(target.join("public")) {
        stdio_error("init", &format!("failed to create public/: {}", err));
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("public").join("index.html"),
        default_public_index_html().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }

    if touched.is_empty() {
        raw("[init] project is already initialized");
        return;
    }

    raw("[init] initialized project files:");
    for path in touched {
        raw(&format!("  - {}", path));
    }
    match run_default_install(&target) {
        Ok(()) => raw("[init] installed default stdlib packages"),
        Err(err) => stdio_error(
            "init",
            &format!(
                "project initialized, but default package install failed: {}",
                err
            ),
        ),
    }
    raw("[init] note: add more packages with `deka add <package>`");
}

fn run_default_install(target: &Path) -> Result<(), String> {
    let previous = std::env::current_dir().map_err(|err| err.to_string())?;
    std::env::set_current_dir(target)
        .map_err(|err| format!("failed to enter {}: {}", target.display(), err))?;
    let payload = InstallPayload {
        specs: DEFAULT_PHP_PACKAGES.iter().map(|s| s.to_string()).collect(),
        yes: true,
        prompt: false,
        quiet: false,
        rehash: false,
    };
    let runtime = tokio::runtime::Runtime::new().map_err(|err| err.to_string())?;
    let result = runtime.block_on(run_install(payload));
    let _ = std::env::set_current_dir(previous);
    result.map_err(|err| err.to_string())
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

fn default_deka_json(name: &str) -> String {
    format!(
        "{{\n  \"name\": \"{}\",\n  \"type\": \"serve\",\n  \"serve\": {{ \"entry\": \"app/main.ds\", \"mode\": \"php\" }},\n  \"tasks\": {{ \"dev\": \"deka serve --dev\" }},\n  \"security\": {{\n    \"allow\": {{}},\n    \"deny\": {{}},\n    \"prompt\": true\n  }}\n}}\n",
        name
    )
}

fn default_deka_lock_json() -> String {
    "{\n  \"lockfileVersion\": 1,\n  \"packages\": {}\n}\n".to_string()
}

fn default_app_page_ds() -> &'static str {
    "export fn Page(): string {\n    return \"<section class=\\\"p-8\\\">\\n  <h1>Deka App</h1>\\n  <p>Project initialized. Edit <code>app/page.ds</code>.</p>\\n</section>\";\n}\n"
}

fn default_main_ds() -> &'static str {
    "export fn App(request: Object): Object {\n    return {\n        status: 200,\n        headers: { 'content-type': 'text/html; charset=utf-8' },\n        body: \"<!doctype html>\\n<html lang=\\\"en\\\">\\n<body>\\n  <main id=\\\"app\\\">Deka App</main>\\n</body>\\n</html>\",\n    };\n}\n"
}

fn default_app_layout_ds() -> &'static str {
    "export fn Layout(props: Object): string {\n    return \"<html lang=\\\"en\\\">\\n<head>\\n  <meta charset=\\\"utf-8\\\" />\\n  <meta name=\\\"viewport\\\" content=\\\"width=device-width, initial-scale=1\\\" />\\n  <title>Deka App</title>\\n</head>\\n<body>\\n  <main id=\\\"app\\\">\" + props.children + \"</main>\\n</body>\\n</html>\";\n}\n"
}

fn default_public_index_html() -> &'static str {
    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>Deka</title>\n  </head>\n  <body>\n    <!-- Static shell only. `deka serve` executes main.ds. -->\n    <div id=\"app\"></div>\n  </body>\n</html>\n"
}

#[cfg(test)]
mod tests {
    use super::default_main_ds;

    #[test]
    fn default_main_uses_dekascript_page_layout_entry() {
        let template = default_main_ds();
        assert!(template.contains("export fn App(request: Object): Object"));
        assert!(template.contains("body: \"<!doctype html>\\n<html"));
        assert!(!template.contains('$'));
        assert!(!template.contains("component/router"));
    }
}
