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

const DEFAULT_DEKA_PHP: &str = include_str!("../../../../php_modules/deka.php");

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
        &target.join("app").join("main.phpx"),
        default_main_phpx().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("app").join("page.phpx"),
        default_app_page_phpx().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("app").join("layout.phpx"),
        default_app_layout_phpx().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }

    if let Err(err) = std::fs::create_dir_all(target.join("php_modules")) {
        stdio_error("init", &format!("failed to create php_modules/: {}", err));
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("php_modules").join("deka.php"),
        DEFAULT_DEKA_PHP.to_string(),
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
        Ok(()) => raw("[init] installed default stdlib packages from LinkHash"),
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
        "{{\n  \"name\": \"{}\",\n  \"type\": \"serve\",\n  \"serve\": {{ \"entry\": \"app/main.phpx\", \"mode\": \"php\" }},\n  \"tasks\": {{ \"dev\": \"deka serve --dev\" }},\n  \"security\": {{\n    \"allow\": {{}},\n    \"deny\": {{}},\n    \"prompt\": true\n  }}\n}}\n",
        name
    )
}

fn default_deka_lock_json() -> String {
    "{\n  \"lockfileVersion\": 1,\n  \"packages\": {}\n}\n".to_string()
}

fn default_app_page_phpx() -> &'static str {
    "export function Page(): string {\n    return \"<section class=\\\"p-8\\\">\\n  <h1>Deka App</h1>\\n  <p>Project initialized. Edit <code>app/page.phpx</code>.</p>\\n</section>\";\n}\n"
}

fn default_main_phpx() -> &'static str {
    "import { Layout } from './layout.phpx';\nimport { Page } from './page.phpx';\n\nfunction request_path($req: mixed): string {\n    if (isset($_SERVER['PATH_INFO'])) {\n        return normalize_path($_SERVER['PATH_INFO']);\n    }\n    if (isset($_SERVER['REQUEST_URI'])) {\n        return normalize_path($_SERVER['REQUEST_URI']);\n    }\n    if (is_array($req) && array_key_exists('url', $req)) {\n        return normalize_path($req['url']);\n    }\n    if (is_object($req) && isset($req.url)) {\n        return normalize_path($req.url);\n    }\n    return '/';\n}\n\nfunction normalize_path($value: mixed): string {\n    $path = '' . $value;\n    $parts = explode('?', $path, 2);\n    $path = $parts[0];\n    if (strpos($path, '://') !== false) {\n        $segments = explode('/', $path, 4);\n        $path = count($segments) >= 4 ? '/' . $segments[3] : '/';\n    }\n    if ($path === '') return '/';\n    return $path;\n}\n\nfunction App($req: mixed) {\n    $path = request_path($req);\n    if ($path !== '/') {\n        return {\n            status: 404,\n            headers: { 'content-type': 'text/plain; charset=utf-8' },\n            body: 'Not Found',\n        };\n    }\n    return {\n        status: 200,\n        headers: { 'content-type': 'text/html; charset=utf-8' },\n        body: \"<!doctype html>\\n\" . Layout({ children: Page() }),\n    };\n}\n\n$app = App;\n"
}

fn default_app_layout_phpx() -> &'static str {
    "export function Layout($props: mixed): string {\n    $children = '';\n    if (is_array($props) && array_key_exists('children', $props)) {\n        $children = '' . $props['children'];\n    } else if (is_object($props) && isset($props.children)) {\n        $children = '' . $props.children;\n    }\n\n    return \"<html lang=\\\"en\\\">\\n<head>\\n  <meta charset=\\\"utf-8\\\" />\\n  <meta name=\\\"viewport\\\" content=\\\"width=device-width, initial-scale=1\\\" />\\n  <title>Deka App</title>\\n</head>\\n<body>\\n  <main id=\\\"app\\\">\" . $children . \"</main>\\n</body>\\n</html>\";\n}\n"
}

fn default_public_index_html() -> &'static str {
    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>Deka</title>\n  </head>\n  <body>\n    <!-- Static shell only. `deka serve` executes main.phpx. -->\n    <div id=\"app\"></div>\n  </body>\n</html>\n"
}

#[cfg(test)]
mod tests {
    use super::default_main_phpx;

    #[test]
    fn default_main_uses_explicit_page_layout_entry() {
        let template = default_main_phpx();
        assert!(template.contains("import { Layout } from './layout.phpx';"));
        assert!(template.contains("import { Page } from './page.phpx';"));
        assert!(template.contains("Layout({ children: Page() })"));
        assert!(template.contains("body: \"<!doctype html>\\n\""));
        assert!(template.contains("if ($path !== '/')"));
        assert!(!template.contains("component/router"));
    }
}
