#![allow(clippy::all, dead_code, unused_imports)]

use deno_core::Extension;

pub mod compiler_api;
pub mod integrity;
pub mod modules;
pub mod seam_contract;
pub mod validation;

pub fn php_extension() -> Extension {
    modules::php::init()
}

pub fn extensions() -> Vec<Extension> {
    vec![modules::php::init()]
}
