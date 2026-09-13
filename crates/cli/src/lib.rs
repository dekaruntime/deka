#![allow(clippy::all, dead_code, unused_variables, unused_assignments)]

use core::{Registry, RegistryBuilder};
#[cfg(target_arch = "wasm32")]
use serde::{Deserialize, Serialize};
use wasm_cli as wasm_cmd;

pub mod cli;
pub mod context;

pub fn build_registry() -> Registry {
    // Registration order matches the pre-move cli crate so `--help` grouping
    // within each category stays byte-identical (BTreeMap by category, then
    // insertion order).
    let mut builder = RegistryBuilder::new()
        .with(cli::register_global_flags)
        .with(cli::register_global_params)
        .with(pm::register_init)
        .with(wasm_cmd::register);

    #[cfg(target_arch = "wasm32")]
    {
        builder = builder.with(deka_db::register);
    }

    #[cfg(feature = "native")]
    {
        builder = builder
            .with(deka_registry::auth::register)
            .with(deka_build::register)
            .with(deka_cache::register)
            .with(compiler::register_check)
            .with(deka_deploy::register)
            .with(compiler::register_fmt)
            .with(compile::register)
            .with(deka_db::register)
            .with(pm::register_install)
            .with(pm::register_summon)
            .with(pm::register_link);
        #[cfg(feature = "lsp")]
        {
            builder = builder.with(compiler::register_lsp);
        }
        builder = builder
            .with(pm::register_pkg)
            .with(deka_registry::publish::register)
            .with(pm::register_release)
            .with(runtime::register_run)
            .with(runtime::register_platform)
            .with(runtime::register_serve)
            .with(dev::register)
            .with(self_cmd::register)
            .with(deka_task::register)
            .with(deka_test::register)
            .with(compiler::register_transpile)
            .with(runtime_core::register)
            .with(introspect::register);
    }

    builder
        .build()
        .unwrap_or_else(|err| panic!("cli registry: {err}"))
}

pub fn run() {
    let registry = build_registry();
    let code = cli::execute(&registry);
    if code != 0 {
        std::process::exit(code);
    }
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

    if cli::single_command_wants_help(cmd) {
        cli::help(&registry);
        let output = stdio::end_capture();
        return WasmRunOutput { code: 0, output };
    }

    if cmd.commands.is_empty() {
        if cmd.flags.is_empty() {
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
    let handler = match ::run::handler::HandlerSnapshot::from_positionals(&cmd.positionals) {
        Ok(handler) => handler,
        Err(_) => match ::run::handler::resolve_handler_path(".") {
            Ok(resolved) => {
                let static_config = ::serve::config::StaticServeConfig::load(&resolved.directory);
                ::run::handler::HandlerSnapshot {
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

    let mut context = core::Context::new(cmd.clone());
    context.env = env;
    context.extensions_mut().insert(handler);

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
