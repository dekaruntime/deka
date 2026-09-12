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
    for directory in ["api", "src"] {
        if let Err(err) = std::fs::create_dir_all(target.join(directory)) {
            stdio_error("init", &format!("failed to create {directory}/: {err}"));
            return;
        }
    }
    if let Err(err) = std::fs::create_dir_all(target.join("public")) {
        stdio_error("init", &format!("failed to create public/: {}", err));
        return;
    }
    if let Err(err) = ensure_file(
        &target.join("public").join("index.html"),
        default_index_html().to_string(),
        &mut touched,
    ) {
        stdio_error("init", &err);
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
        "{{\n  \"name\": \"{}\",\n  \"type\": \"serve\",\n  \"serve\": {{ \"mode\": \"static\", \"entry\": \"public\" }},\n  \"tasks\": {{ \"dev\": \"deka serve --dev\" }},\n  \"security\": {{\n    \"allow\": {{}},\n    \"deny\": {{}},\n    \"prompt\": true\n  }}\n}}\n",
        name
    )
}

fn default_deka_lock_json() -> String {
    "{\n  \"lockfileVersion\": 1,\n  \"packages\": {}\n}\n".to_string()
}

fn default_index_html() -> &'static str {
    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>Deka</title>\n    <link rel=\"stylesheet\" href=\"/style.css\" />\n  </head>\n  <body>\n    <main><h1>Deka App</h1><p>Project initialized.</p></main>\n  </body>\n</html>\n"
}

fn default_public_style_css() -> &'static str {
    "body { font-family: system-ui, sans-serif; margin: 2rem; }\n"
}

#[cfg(test)]
mod tests {
    use super::{default_deka_json, default_index_html};

    #[test]
    fn default_scaffold_serves_public_html() {
        let json: serde_json::Value = serde_json::from_str(&default_deka_json("demo")).unwrap();
        assert_eq!(json["serve"]["mode"], "static");
        assert_eq!(json["serve"]["entry"], "public");
        let index = default_index_html();
        assert!(index.contains("<h1>Deka App</h1>"));
        assert!(index.contains("href=\"/style.css\""));
        assert!(!index.contains("<!--deka-"));
    }
}
