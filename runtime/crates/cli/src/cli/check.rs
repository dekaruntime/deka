use core::{CommandSpec, Context, Registry};
use phpx_js::{compile_phpx_source_to_js, parse_source_module_meta};
use std::fs;
use std::path::Path;

const COMMAND: CommandSpec = CommandSpec {
    name: "check",
    category: "project",
    summary: "validate a DekaScript or legacy PHPX source file",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

pub fn cmd(context: &Context) {
    if let Err(err) = run(context) {
        stdio::error("check", &err);
        std::process::exit(1);
    }
}

fn run(context: &Context) -> Result<(), String> {
    let input = context
        .args
        .positionals
        .first()
        .ok_or_else(|| "usage: deka check <file.ds>".to_string())?;
    let path = Path::new(input);
    if !is_deka_source_path(path) {
        return Err(format!(
            "DekaScript uses .ds only; migrate '{}' before checking it",
            input
        ));
    }

    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    compile_phpx_source_to_js(&source, input, parse_source_module_meta(&source))?;
    stdio::success(&format!("checked {}", path.display()));
    Ok(())
}

fn is_deka_source_path(path: &Path) -> bool {
    matches!(path.extension().and_then(|ext| ext.to_str()), Some("ds"))
}

#[cfg(test)]
mod tests {
    use super::is_deka_source_path;
    use std::path::Path;

    #[test]
    fn accepts_dekascript_only() {
        assert!(is_deka_source_path(Path::new("main.ds")));
        assert!(!is_deka_source_path(Path::new("main.phpx")));
        assert!(!is_deka_source_path(Path::new("main.ts")));
    }
}
