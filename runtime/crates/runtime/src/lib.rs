#![allow(clippy::all)]

use core::Context;

mod env;
mod extensions;
mod js_pipeline;
mod platform;
mod run;
mod security;
mod serve;

pub fn run(context: &Context) {
    run::run(context);
}

pub fn serve(context: &Context) {
    serve::serve(context);
}

pub fn platform(context: &Context) {
    platform::platform(context);
}

pub fn serve_desktop(context: &Context) {
    let _ = context;
    eprintln!("[desktop] desktop runtime mode is deferred in reboot MVP");
    std::process::exit(1);
}

pub fn has_embedded_vfs() -> bool {
    false
}
