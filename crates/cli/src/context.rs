//! Runtime handler state is captured once, before command dispatch.
use core::{Context, ParseError, Registry};
use run::handler::{HandlerSnapshot, resolve_handler_path};

#[derive(Debug)]
pub enum ContextError {
    Parse(Vec<ParseError>),
    HandlerResolve(String),
}

pub fn from_env(registry: &Registry) -> Result<Context, ContextError> {
    let parsed = core::parse_env(registry);
    if !parsed.errors.is_empty() {
        return Err(ContextError::Parse(parsed.errors));
    }
    prepare(Context::new(parsed.args))
}

fn prepare(mut context: Context) -> Result<Context, ContextError> {
    let handler = match HandlerSnapshot::from_positionals(&context.args.positionals) {
        Ok(handler) => handler,
        Err(message) => {
            if context
                .args
                .commands
                .iter()
                .any(|cmd| matches!(cmd.as_str(), "test" | "self" | "link" | "unlink" | "pkg"))
            {
                let resolved = resolve_handler_path(".").map_err(ContextError::HandlerResolve)?;
                HandlerSnapshot {
                    input: ".".to_string(),
                    resolved,
                }
            } else {
                return Err(ContextError::HandlerResolve(message));
            }
        }
    };

    context.extensions_mut().insert(handler);
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn context(command: &str, path: &std::path::Path) -> Context {
        Context::new(core::Args {
            flags: HashMap::new(),
            params: HashMap::new(),
            commands: vec![command.into()],
            positionals: vec![path.to_string_lossy().into_owned()],
        })
    }

    #[test]
    fn dispatch_snapshot_survives_config_change_and_artifact_rewrite() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("original.js"), "").unwrap();
        std::fs::write(root.path().join("artifact.js"), "").unwrap();
        let config = root.path().join("deka.json");
        std::fs::write(&config, r#"{"serve":{"entry":"original.js"}}"#).unwrap();
        let original = prepare(context("serve", root.path())).unwrap();
        std::fs::write(&config, r#"{"serve":{"entry":"missing.js"}}"#).unwrap();
        let snapshot = original.extensions().get::<HandlerSnapshot>().unwrap();
        assert!(snapshot.resolved.path.ends_with("original.js"));
        #[cfg(feature = "native")]
        {
            let rewritten = deka_cache::rewrite_context_for_artifact(
                &original,
                &root.path().join("artifact.js"),
            )
            .unwrap();
            assert!(
                rewritten
                    .extensions()
                    .get::<HandlerSnapshot>()
                    .unwrap()
                    .resolved
                    .path
                    .ends_with("artifact.js")
            );
            assert!(
                original
                    .extensions()
                    .get::<HandlerSnapshot>()
                    .unwrap()
                    .resolved
                    .path
                    .ends_with("original.js")
            );
            assert_eq!(original.args.positionals[0], root.path().to_string_lossy());
        }
    }

    #[test]
    fn dispatch_keeps_resolution_errors_and_command_fallbacks() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("deka.json"), r#"{"serve":{"entry":"missing.js"}}"#)
            .unwrap();
        assert!(matches!(
            prepare(context("serve", root.path())),
            Err(ContextError::HandlerResolve(_))
        ));
        for command in ["test", "self", "link", "unlink", "pkg"] {
            let prepared = prepare(context(command, root.path())).unwrap();
            assert_eq!(
                prepared
                    .extensions()
                    .get::<HandlerSnapshot>()
                    .unwrap()
                    .input,
                "."
            );
        }
    }

    // deka#1038: a leftover serve.json must not be reachable through the
    // "test"/"self"/"link"/"unlink"/"pkg" resolution-error fallback either
    // -- the migration error is a hard stop for the explicit path, and the
    // fallback resolves "." against the real process cwd, so this only
    // pins that the explicit-path branch surfaces the migration error
    // rather than silently swallowing it into the fallback path.
    #[test]
    fn dispatch_surfaces_serve_json_migration_error_for_serve_command() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("index.html"), "<html></html>").unwrap();
        std::fs::write(root.path().join("serve.json"), r#"{"entry":"index.html"}"#).unwrap();
        match prepare(context("serve", root.path())) {
            Err(ContextError::HandlerResolve(message)) => {
                assert!(message.contains("serve.json"), "{message}");
                assert!(message.contains("deka.json"), "{message}");
            }
            other => panic!("expected a serve.json migration error, got {other:?}"),
        }
    }
}
