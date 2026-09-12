use crate::{Args, ParseError, Registry, parse_env};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Consumer-owned state, keyed by its concrete type.
///
/// Populate once in the dispatcher before handing the context to handlers.
/// Cloning copies the map and shares its values through cheap `Arc` clones;
/// inserting a replacement affects only the map being modified.
#[derive(Debug, Clone, Default)]
pub struct Extensions {
    values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Extensions {
    /// Store a value, replacing any previous value of the same concrete type.
    pub fn insert<T: Any + Send + Sync>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Arc::new(value));
    }

    /// Borrow a value for the lifetime of this extension map, without cloning it.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref()
    }
}

/// Parsed input, working directory, and consumer state passed to handlers.
///
/// Cloning shares extension values without requiring them to implement `Clone`.
/// Construct with `Context::new`, then populate extensions before dispatch.
#[derive(Debug, Clone)]
pub struct Context {
    pub args: Args,
    pub env: EnvContext,
    extensions: Extensions,
}

/// Working directory only; loading this context never reads environment variables.
#[derive(Debug, Clone)]
pub struct EnvContext {
    pub cwd: PathBuf,
}

#[derive(Debug, Clone)]
pub enum ContextError {
    Parse(Vec<ParseError>),
}

impl EnvContext {
    /// Capture the process cwd, falling back to `.` if it is unavailable.
    /// On wasm32, use `.` without accessing the host process.
    pub fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        #[cfg(target_arch = "wasm32")]
        let cwd = PathBuf::from(".");
        Self { cwd }
    }
}

impl Context {
    pub fn new(args: Args) -> Self {
        Self {
            args,
            env: EnvContext::load(),
            extensions: Extensions::default(),
        }
    }

    /// Read consumer-owned state populated before handler dispatch.
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Populate consumer-owned state before handler dispatch.
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// Parse process arguments and capture the working directory.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_env(registry: &Registry) -> Result<Self, ContextError> {
        let parsed = parse_env(registry);
        if !parsed.errors.is_empty() {
            return Err(ContextError::Parse(parsed.errors));
        }
        Ok(Self::new(parsed.args))
    }
}
