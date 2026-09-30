//! Adds only the embedded renderer module; all application resolution and policy
//! remain with Deka's existing loader.
use deno_core::error::ModuleLoaderError;
use deno_core::{
    ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader, ModuleSource,
    ModuleSourceCode, ModuleSpecifier, ModuleType, ResolutionKind,
};
use pool::PhpxEsmLoader;
use std::{future::Future, pin::Pin};

pub const ADAPTER: &str = "deka-native:///renderer.js";
pub struct NativeLoader(pub PhpxEsmLoader);
impl ModuleLoader for NativeLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        if specifier == ADAPTER {
            Ok(ModuleSpecifier::parse(ADAPTER).expect("constant renderer URL"))
        } else {
            self.0.resolve(specifier, referrer, kind)
        }
    }
    fn load(
        &self,
        specifier: &ModuleSpecifier,
        referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        if specifier.as_str() == ADAPTER {
            let source = include_str!("../reconciler.js").replace(
                "/*__RECONCILER__*/",
                include_str!("../../vendor/react-reconciler.production.js"),
            );
            ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(source.into()),
                specifier,
                None,
            )))
        } else {
            self.0.load(specifier, referrer, options)
        }
    }
    fn prepare_load(
        &self,
        specifier: &ModuleSpecifier,
        referrer: Option<String>,
        content: Option<String>,
        options: ModuleLoadOptions,
    ) -> Pin<Box<dyn Future<Output = Result<(), ModuleLoaderError>>>> {
        if specifier.as_str() == ADAPTER {
            Box::pin(async { Ok(()) })
        } else {
            self.0.prepare_load(specifier, referrer, content, options)
        }
    }
}
