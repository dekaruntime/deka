use std::path::Path as FsPath;
use std::sync::Arc;

use crate::env::init_env;
use crate::extensions::extensions_for_mode;
use crate::js_pipeline::build_phpx_handler_bundle;
use crate::security::resolve_security_policy;
use core::Context;
use engine::{RuntimeEngine, config as runtime_config, set_engine};
use modules_php::validation::{format_validation_error, modules::validate_module_resolution};
use platform::Platform;
use platform_server::ServerPlatform;
use pool::{ExecutionMode, HandlerKey, PoolConfig, RequestData};
use runtime_core::env::{set_default_log_level_with, set_handler_path_with, set_runtime_args_with};
use runtime_core::handler::{
    handler_input_with, is_html_entry, is_php_entry, normalize_handler_path_with,
};
use runtime_core::modules::ensure_phpx_module_root_env_with;
use runtime_core::process::parse_exit_code;
use runtime_core::validation::validate_phpx_handler_with;

pub fn run(context: &Context) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime");

    if let Err(err) = rt.block_on(run_async(context)) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}

async fn run_async(context: &Context) -> Result<(), String> {
    init_env();
    let platform = ServerPlatform::default();
    let resolved_security = resolve_security_policy(context)?;
    for warning in resolved_security.warnings {
        eprintln!("[security] warning: {}", warning);
    }
    eprintln!("[security] {}", resolved_security.summary);
    let _ = platform
        .env()
        .set("DEKA_SECURITY_POLICY", &resolved_security.policy_json);
    let _ = platform.env().set(
        "DEKA_SECURITY_NO_PROMPT",
        if resolved_security.prompt_enabled {
            "0"
        } else {
            "1"
        },
    );
    let _ = platform.env().set("DEKA_SECURITY_ENFORCE", "1");
    let env_get = |key: &str| platform.env().get(key);
    let mut env_set = |key: &str, value: &str| {
        let _ = platform.env().set(key, value);
    };
    set_default_log_level_with(&env_get, &mut env_set);

    let (handler_path, extra_args) = handler_input_with(&context.args.positionals, &env_get);
    let mut env_set = |key: &str, value: &str| {
        let _ = platform.env().set(key, value);
    };
    set_runtime_args_with(&extra_args, &mut env_set, &|| std::env::args().next());

    let mut env_set = |key: &str, value: &str| {
        let _ = platform.env().set(key, value);
    };
    set_handler_path_with(&handler_path, &env_get, &mut env_set);

    let normalized =
        normalize_handler_path_with(&handler_path, &|| platform.fs().cwd().ok(), &|path| {
            platform.fs().canonicalize(path).ok()
        });
    if is_html_entry(&normalized) {
        return Err(format!(
            "Run mode does not support HTML entrypoints: {}",
            normalized
        ));
    }

    if !is_php_entry(&normalized) {
        return Err(format!(
            "Run mode in reboot MVP only supports .phpx entrypoints: {}",
            normalized
        ));
    }
    let mut env_set = |key: &str, value: &str| {
        let _ = platform.env().set(key, value);
        unsafe { std::env::set_var(key, value) };
    };
    ensure_phpx_module_root_env_with(
        &normalized,
        &|path| platform.fs().exists(path),
        &|| platform.fs().current_exe().ok(),
        &env_get,
        &mut env_set,
    );
    validate_phpx_modules(&normalized)?;
    let serve_mode = runtime_config::ServeMode::Php;

    let _ = std::fs::read_to_string(&normalized)
        .map_err(|err| format!("Failed to read handler from {}: {}", normalized, err))?;

    let use_esm = std::env::var("DEKA_RUNTIME_ESM")
        .map(|value| value != "0" && value != "false")
        .unwrap_or(true);
    let handler_code = if use_esm {
        String::new()
    } else {
        build_phpx_handler_bundle(&normalized)?
    };

    let runtime_cfg = runtime_config::RuntimeConfig::load();
    let mut pool_config = PoolConfig::from_env();
    // Run mode should allow long-lived servers without timing out.
    pool_config.request_timeout_ms = 0;
    if let Some(enabled) = runtime_cfg.code_cache_enabled() {
        pool_config.enable_code_cache = enabled;
    }

    let serve_mode_for_extensions = serve_mode.clone();
    let extensions_provider = Arc::new(move || extensions_for_mode(&serve_mode_for_extensions));

    let engine = Arc::new(RuntimeEngine::new(
        pool_config.clone(),
        pool_config,
        &runtime_cfg,
        extensions_provider,
    ));
    let _ = set_engine(Arc::clone(&engine));

    let handler_key = HandlerKey::new(
        FsPath::new(&normalized)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&normalized),
    );

    let request_value = serde_json::json!({
        "url": "http://localhost/run",
        "method": "GET",
        "headers": {},
        "body": "",
    });
    let execution_mode = ExecutionMode::Module;

    let response = engine
        .execute(
            handler_key,
            RequestData {
                handler_code,
                handler_entry: Some(normalized.clone()),
                request_value,
                request_parts: None,
                mode: execution_mode,
            },
        )
        .await
        .map_err(|err| format!("Run failed: {}", err))?;

    if !response.success {
        if let Some(error) = response.error {
            if let Some(code) = parse_exit_code(&error) {
                std::process::exit(code);
            }
            return Err(format!("Run failed: {}", error));
        }
        return Err("Run failed: unknown error".to_string());
    }

    if let Some(result) = response.result.as_ref() {
        if let Some(code) = result.get("exit_code").and_then(|value| value.as_i64()) {
            std::process::exit(code as i32);
        }
    }

    // The runtime hold promise keeps long-lived run-mode handlers alive.
    Ok(())
}

fn validate_phpx_modules(handler_path: &str) -> Result<(), String> {
    validate_phpx_handler_with(
        handler_path,
        &|path| {
            std::fs::read_to_string(path)
                .map_err(|err| format!("Failed to read PHPX handler {}: {}", path, err))
        },
        &|source, path| validate_module_resolution(source, path),
        &|source, path, error| format_validation_error(source, path, error),
    )
}
