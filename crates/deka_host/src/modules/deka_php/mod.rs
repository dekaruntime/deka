// Minimal PHP prelude module
use deno_core::Extension;

deno_core::extension!(
    deka_php,
    esm_entry_point = "ext:deka_php/php.js",
    esm = [dir "src/modules/deka_php", "php.js"],
);

pub fn init() -> Extension {
    deka_php::init()
}
