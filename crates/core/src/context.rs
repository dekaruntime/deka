use std::collections::HashMap;
use std::path::PathBuf;

use crate::args::{Args, ParseError, parse_env};
use crate::handler::{ResolvedHandler, StaticServeConfig, resolve_handler_path};
use crate::registry::Registry;

#[derive(Debug, Clone)]
pub struct Context {
    pub args: Args,
    pub env: EnvContext,
    pub handler: HandlerContext,
}

#[derive(Debug, Clone)]
pub struct EnvContext {
    pub vars: HashMap<String, String>,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone)]
pub struct HandlerContext {
    pub input: String,
    pub resolved: ResolvedHandler,
    pub static_config: StaticServeConfig,
    pub serve_config_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum ContextError {
    Parse(Vec<ParseError>),
    HandlerResolve(String),
}

impl EnvContext {
    pub fn load() -> Self {
        let vars = std::env::vars().collect::<HashMap<_, _>>();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self { vars, cwd }
    }
}

impl HandlerContext {
    pub fn from_env(args: &Args) -> Result<Self, String> {
        let input = args
            .positionals
            .first()
            .cloned()
            .or_else(|| std::env::var("HANDLER_PATH").ok())
            .unwrap_or_else(|| ".".to_string());

        let resolved = resolve_handler_path(&input)?;
        let static_config = StaticServeConfig::load(&resolved.directory);
        let serve_config_path = resolved.directory.join("serve.json");

        Ok(Self {
            input,
            resolved,
            static_config,
            serve_config_path: serve_config_path.exists().then_some(serve_config_path),
        })
    }
}

impl Context {
    pub fn from_env(registry: &Registry) -> Result<Self, ContextError> {
        let parsed = parse_env(registry);
        if !parsed.errors.is_empty() {
            return Err(ContextError::Parse(parsed.errors));
        }

        let env = EnvContext::load();
        let handler = match HandlerContext::from_env(&parsed.args) {
            Ok(handler) => handler,
            Err(message) => {
                if parsed
                    .args
                    .commands
                    .iter()
                    .any(|cmd| matches!(cmd.as_str(), "test" | "self" | "link" | "unlink" | "pkg"))
                {
                    let resolved =
                        resolve_handler_path(".").map_err(ContextError::HandlerResolve)?;
                    let static_config = StaticServeConfig::load(&resolved.directory);
                    let serve_config_path = resolved.directory.join("serve.json");
                    HandlerContext {
                        input: ".".to_string(),
                        resolved,
                        static_config,
                        serve_config_path: serve_config_path.exists().then_some(serve_config_path),
                    }
                } else {
                    return Err(ContextError::HandlerResolve(message));
                }
            }
        };

        Ok(Self {
            args: parsed.args,
            env,
            handler,
        })
    }
}
