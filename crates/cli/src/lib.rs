#![allow(clippy::all, dead_code, unused_variables, unused_assignments)]

use core::{Registry, RegistryBuilder};
#[cfg(target_arch = "wasm32")]
use serde::{Deserialize, Serialize};
use wasm_cli as wasm_cmd;

pub mod cli;
pub mod context;

/// The ordered list of registration functions that make up the CLI.
/// Registration order matches the pre-move cli crate so `--help` grouping
/// within each category stays byte-identical (BTreeMap by category, then
/// insertion order). [`build_registry`] folds these into the real registry;
/// [`command_flag_index`] re-runs each one against a private scratch
/// registry to learn exactly which flags/params it — and only it —
/// registers, which is how per-command help resolves a flag by its actual
/// owner instead of the first same-named entry anywhere in the registry
/// (deka#996 review).
pub fn register_fns() -> Vec<fn(&mut Registry)> {
    let mut fns: Vec<fn(&mut Registry)> = vec![
        cli::register_global_flags,
        cli::register_global_params,
        pm::register_init,
        wasm_cmd::register,
    ];

    #[cfg(target_arch = "wasm32")]
    {
        fns.push(deka_db::register);
    }

    #[cfg(feature = "native")]
    {
        fns.extend([
            deka_registry::auth::register,
            deka_build::register,
            deka_cache::register,
            compiler::register_check,
            deka_deploy::register,
            compiler::register_fmt,
            compile::register,
            deka_db::register,
            pm::register_install,
            pm::register_summon,
            pm::register_link,
        ]);
        #[cfg(feature = "lsp")]
        {
            fns.push(compiler::register_lsp);
        }
        fns.extend([
            pm::register_pkg,
            deka_registry::publish::register,
            pm::register_release,
            runtime::register_run,
            runtime::register_platform,
            runtime::register_serve,
            self_cmd::register,
            deka_task::register,
            deka_test::register,
            compiler::register_transpile,
            runtime_core::register,
            introspect::register,
        ]);
    }

    #[cfg(feature = "native")]
    {
        fns.push(dev::register);
    }

    fns
}

pub fn build_registry() -> Registry {
    let mut builder = RegistryBuilder::new();
    for register_fn in register_fns() {
        builder = builder.with(register_fn);
    }
    builder
        .build()
        .unwrap_or_else(|err| panic!("cli registry: {err}"))
}

/// Per-command flag/param ownership for help rendering. See
/// [`core::help::build_ownership_index`] for how this avoids the
/// first-match-wins bug a name-only lookup against the shared registry
/// would have (deka#996 review).
pub fn command_flag_index() -> core::help::OwnershipIndex {
    core::help::build_ownership_index(&register_fns())
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
    let ownership_index = command_flag_index();
    stdio::begin_capture();

    let parsed = core::Args::collect(args, &registry);
    if !parsed.errors.is_empty() {
        let message = cli::format_parse_errors(&ownership_index, &parsed.errors);
        cli::error(Some(message.as_str()));
        let output = stdio::end_capture();
        return WasmRunOutput { code: 1, output };
    }

    let cmd = &parsed.args;

    if cli::single_command_wants_help(cmd) {
        match registry.command_named(&cmd.commands[0]) {
            Some(command) => {
                let index = command_flag_index();
                cli::command_help(command, index.flags.get(command.name));
            }
            None => cli::help(&registry),
        }
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
    }

    let env = core::EnvContext::load();
    let handler = match ::run::handler::HandlerSnapshot::from_positionals(&cmd.positionals) {
        Ok(handler) => handler,
        Err(_) => match ::run::handler::resolve_handler_path(".").and_then(|resolved| {
            ::serve::config::StaticServeConfig::load(&resolved.directory)
                .map(|static_config| (resolved, static_config))
        }) {
            Ok((resolved, static_config)) => ::run::handler::HandlerSnapshot {
                input: ".".to_string(),
                resolved,
                static_config,
                serve_config_path: None,
            },
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
