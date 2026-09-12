use std::path::PathBuf;
use std::sync::Arc;

use core::Context;
use pool::PoolConfig;

pub fn serve(context: &Context) {
    serve_with_dsc(context, None);
}

pub fn serve_with_dsc(context: &Context, dsc: Option<PathBuf>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime");

    if let Err(err) = rt.block_on(serve_async(context, dsc)) {
        stdio::error("serve", &err);
        std::process::exit(1);
    }
}

async fn serve_async(context: &Context, dsc: Option<PathBuf>) -> Result<(), String> {
    crate::banner::prepare(
        true,
        &context
            .extensions()
            .get::<::run::handler::HandlerSnapshot>()
            .expect("handler snapshot populated before dispatch")
            .input,
    )?;

    let resolved_security = runtime::security::resolve_security_policy_for_serve(context, true)?;
    let mut serve_options = pool::validation::ServeOptions::default();
    runtime::apply_cli_serve_overrides(context, &mut serve_options);
    let mut pool_config: PoolConfig = runtime::configure_pool(&serve_options, dsc);
    pool_config.dev_mode = true;
    pool_config.enable_code_cache = false;

    let prepared =
        runtime::prepare_http_session(context, resolved_security, pool_config, serve_options)?;
    stdio::log("dev", "enabled");

    if let Err(err) = crate::watch::start_watch(
        &prepared.handler_path,
        Arc::clone(&prepared.state.engine),
        true,
    ) {
        tracing::warn!("watch mode failed: {}", err);
    }

    runtime::bind_and_listen(prepared, Some(crate::banner::print_banner)).await
}
