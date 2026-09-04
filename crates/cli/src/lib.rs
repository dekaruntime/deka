#![allow(clippy::all, dead_code, unused_variables, unused_assignments)]

use core::Registry;
#[cfg(target_arch = "wasm32")]
use serde::{Deserialize, Serialize};
use wasm_cli as wasm_cmd;

pub mod cli;
pub mod compile_helper;

pub fn build_registry() -> Registry {
    let mut registry = Registry::new();
    cli::register_global_flags(&mut registry);
    cli::register_global_params(&mut registry);
    cli::init::register(&mut registry);
    cli::user::register(&mut registry);
    wasm_cmd::register(&mut registry);
    #[cfg(target_arch = "wasm32")]
    cli::db_wasm::register(&mut registry);
    #[cfg(feature = "native")]
    {
        cli::auth::register(&mut registry);
        cli::build::register(&mut registry);
        cli::check::register(&mut registry);
        cli::deploy::register(&mut registry);
        cli::fmt::register(&mut registry);
        cli::compile::register(&mut registry);
        cli::db::register(&mut registry);
        cli::install::register(&mut registry);
        cli::link::register(&mut registry);
        #[cfg(feature = "lsp")]
        cli::lsp::register(&mut registry);
        cli::pkg::register(&mut registry);
        cli::publish::register(&mut registry);
        cli::release::register(&mut registry);
        cli::run::register(&mut registry);
        cli::platform::register(&mut registry);
        cli::serve::register(&mut registry);
        cli::self_cmd::register(&mut registry);
        cli::task::register(&mut registry);
        cli::test::register(&mut registry);
        cli::transpile::register(&mut registry);
        introspect::register(&mut registry);
    }
    registry
}

pub fn run() {
    let registry = build_registry();
    cli::execute(&registry);
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Deserialize)]
struct WasmRunInput {
    #[serde(default)]
    args: Vec<String>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Serialize)]
struct WasmRunOutput {
    code: i32,
    output: String,
}

#[cfg(target_arch = "wasm32")]
fn run_for_wasm(args: Vec<String>) -> WasmRunOutput {
    let registry = build_registry();
    stdio::begin_capture();

    let parsed = core::Args::collect(args, &registry);
    if !parsed.errors.is_empty() {
        let message = cli::format_parse_errors(&parsed.errors);
        cli::error(Some(message.as_str()));
        let output = stdio::end_capture();
        return WasmRunOutput { code: 1, output };
    }

    let cmd = &parsed.args;

    if cmd.commands.is_empty() {
        if cmd.flags.is_empty() {
            cli::help(&registry);
            let output = stdio::end_capture();
            return WasmRunOutput { code: 0, output };
        }
        if cmd.flags.contains_key("--help")
            || cmd.flags.contains_key("-H")
            || cmd.flags.contains_key("help")
        {
            cli::help(&registry);
            let output = stdio::end_capture();
            return WasmRunOutput { code: 0, output };
        }
        if cmd.flags.contains_key("--version")
            || cmd.flags.contains_key("-V")
            || cmd.flags.contains_key("version")
        {
            let verbose = cmd.flags.contains_key("--verbose");
            cli::version(verbose);
            let output = stdio::end_capture();
            return WasmRunOutput { code: 0, output };
        }
        if cmd.flags.contains_key("--update") || cmd.flags.contains_key("-U") {
            cli::update();
            let output = stdio::end_capture();
            return WasmRunOutput { code: 0, output };
        }
    }

    let env = core::EnvContext::load();
    let handler = match core::HandlerContext::from_env(cmd) {
        Ok(handler) => handler,
        Err(_) => match core::resolve_handler_path(".") {
            Ok(resolved) => {
                let static_config = core::StaticServeConfig::load(&resolved.directory);
                core::HandlerContext {
                    input: ".".to_string(),
                    resolved,
                    static_config,
                    serve_config_path: None,
                }
            }
            Err(message) => {
                cli::error(Some(message.as_str()));
                let output = stdio::end_capture();
                return WasmRunOutput { code: 1, output };
            }
        },
    };

    let context = core::Context {
        args: cmd.clone(),
        env,
        handler,
    };

    if cmd.commands.len() > 2 {
        cli::error(None);
        let output = stdio::end_capture();
        return WasmRunOutput { code: 1, output };
    }

    let cmd_name = match cmd.commands.get(0) {
        Some(value) => value,
        None => {
            cli::error(None);
            let output = stdio::end_capture();
            return WasmRunOutput { code: 1, output };
        }
    };

    let Some(command) = registry.command_named(cmd_name) else {
        cli::error(None);
        let output = stdio::end_capture();
        return WasmRunOutput { code: 1, output };
    };

    if cmd.commands.len() == 1 {
        (command.handler)(&context);
        let output = stdio::end_capture();
        return WasmRunOutput { code: 0, output };
    }

    let sub_name = &cmd.commands[1];
    let Some(subcommand) = registry.subcommand_named(command, sub_name) else {
        cli::error(None);
        let output = stdio::end_capture();
        return WasmRunOutput { code: 1, output };
    };

    (subcommand.handler)(&context);
    let output = stdio::end_capture();
    WasmRunOutput { code: 0, output }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn deka_wasm_alloc(size: u32) -> u32 {
    let mut buffer = Vec::<u8>::with_capacity(size as usize);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr as u32
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn deka_wasm_free(ptr: u32, size: u32) {
    if ptr == 0 {
        return;
    }
    unsafe {
        let _ = Vec::from_raw_parts(ptr as *mut u8, 0, size as usize);
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn deka_wasm_run_json(ptr: u32, len: u32) -> u64 {
    let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let input_str = std::str::from_utf8(input_bytes).unwrap_or("{\"args\":[]}");
    let input: WasmRunInput =
        serde_json::from_str(input_str).unwrap_or(WasmRunInput { args: Vec::new() });

    let output = run_for_wasm(input.args);
    let output_bytes = serde_json::to_vec(&output).unwrap_or_else(|_| {
        b"{\"code\":1,\"output\":\"failed to serialize wasm cli output\"}".to_vec()
    });

    let mut boxed = output_bytes.into_boxed_slice();
    let out_ptr = boxed.as_mut_ptr() as u32;
    let out_len = boxed.len() as u32;
    std::mem::forget(boxed);
    ((out_len as u64) << 32) | out_ptr as u64
}
