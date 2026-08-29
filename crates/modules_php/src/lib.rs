#![allow(clippy::all, dead_code, unused_imports)]

#[cfg(feature = "runtime")]
use deno_core::Extension;

#[cfg(feature = "compiler")]
pub mod compiler_api;
#[cfg(feature = "compiler")]
pub mod integrity;
#[cfg(feature = "runtime")]
pub mod modules;
#[cfg(feature = "compiler")]
pub mod validation;

#[cfg(feature = "runtime")]
pub fn php_extension() -> Extension {
    modules::php::init()
}

#[cfg(feature = "runtime")]
pub fn extensions() -> Vec<Extension> {
    vec![modules::php::init()]
}
