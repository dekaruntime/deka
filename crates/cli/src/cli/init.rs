use core::{CommandSpec, Context, Registry};
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
        &target.join("index.html"),
        default_index_html().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("app").join("page.dsx"),
        default_app_page_dsx().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("app").join("layout.dsx"),
        default_app_layout_dsx().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("app").join("not-found.dsx"),
        default_not_found_dsx().to_string(),
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
        &target.join("public").join("style.css"),
        default_public_style_css().to_string(),
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
    raw("[init] note: add packages with `deka add <package>`");
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
        "{{\n  \"name\": \"{}\",\n  \"type\": \"serve\",\n  \"serve\": {{ \"mode\": \"ds\" }},\n  \"tasks\": {{ \"dev\": \"deka serve --dev\" }},\n  \"security\": {{\n    \"allow\": {{}},\n    \"deny\": {{}},\n    \"prompt\": true\n  }}\n}}\n",
        name
    )
}

fn default_deka_lock_json() -> String {
    "{\n  \"lockfileVersion\": 1,\n  \"packages\": {}\n}\n".to_string()
}

fn default_app_page_dsx() -> &'static str {
    "export fn Page() {\n    return <section><h1>Deka App</h1><p>Project initialized.</p></section>;\n}\n"
}

fn default_app_layout_dsx() -> &'static str {
    "interface LayoutProps { children: Component }\nexport fn Layout(props: LayoutProps) {\n    return <main>{props.children}</main>;\n}\n"
}

fn default_not_found_dsx() -> &'static str {
    "export fn Page() {\n    return <section><h1>Not found</h1></section>;\n}\n"
}

fn default_index_html() -> &'static str {
    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>Deka</title>\n    <link rel=\"stylesheet\" href=\"/style.css\" />\n    <!--deka-head-->\n  </head>\n  <body>\n    <!--deka-app-->\n    <!--deka-scripts-->\n  </body>\n</html>\n"
}

fn default_public_style_css() -> &'static str {
    "body { font-family: system-ui, sans-serif; margin: 2rem; }\n"
}

#[cfg(test)]
mod tests {
    use super::{default_app_page_dsx, default_deka_json, default_index_html};

    #[test]
    fn default_scaffold_is_the_rfd_document() {
        let json = default_deka_json("demo");
        assert!(!json.contains("serve.entry"));
        assert!(!json.contains("app/main.ds"));
        let page = default_app_page_dsx();
        assert!(page.contains("export fn Page()"));
        assert!(page.contains("<section>"));
        assert!(!page.contains("string {"));
        let index = default_index_html();
        assert!(index.contains("<!--deka-head-->"));
        assert!(index.contains("<!--deka-app-->"));
        assert!(index.contains("<!--deka-scripts-->"));
        assert!(!index.contains("id=\"app\""));
    }
}
