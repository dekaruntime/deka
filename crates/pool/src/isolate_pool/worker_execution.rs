use super::*;

/// Bind host dispatchers as locals so PHPX/DS emit can name `__bridge` /
/// `__deka_host` without those identifiers living on user `globalThis`.
fn wrap_with_host_bindings(body: &str) -> String {
    format!(
        "(function() {{\nconst __h = globalThis[Symbol.for('deka.host.internal')];\n(function(__deka_host, __bridge, __deka_to_result) {{\n{body}\n}})(__h && __h.host, __h && __h.bridge, __h && __h.toResult);\n}})();"
    )
}


impl WorkerThread {
    /// Execute a request in the warm isolate
    pub(super) async fn execute_in_isolate(
        &mut self,
        key: &HandlerKey,
        request: &WorkerRequest,
    ) -> (ExecutionOutcome, ExecutionProfile) {
        // Get mutable reference to isolate
        let use_code_cache = self.config.enable_code_cache;
        let secrets_cache = Arc::clone(&self.secrets_cache);
        let (isolates, code_cache) = (&mut self.isolates, &mut self.code_cache);
        let isolate = isolates
            .get_mut(key)
            .ok_or_else(|| "Isolate not found".to_string());

        let isolate = match isolate {
            Ok(isolate) => isolate,
            Err(err) => return (ExecutionOutcome::Err(err), ExecutionProfile::empty()),
        };

        // Ensure THIS isolate is the currently-entered one on this thread.
        //
        // Each `OwnedIsolate` in deno_core is `Enter()`ed at construction and
        // `Exit()`ed on drop, so with N isolates on one worker only the most
        // recently constructed one is `v8__Isolate__GetCurrent()`. When we
        // dispatch a request to an older isolate the current-isolate mismatch
        // makes `ContextScope::new` panic with
        //   "PinnedRef<HandleScope<()>> and Context do not belong to the same Isolate".
        //
        // V8 allows re-entering an already-entered isolate; the matching
        // `Exit()` simply restores the previous top-of-stack. `_enter_guard`
        // does the exit on drop (covering every early return in this method
        // without having to thread cleanup through each match arm).
        let _enter_guard = IsolateEntryGuard::enter(isolate.runtime.v8_isolate());

        isolate.active_requests = 1;
        isolate.state = IsolateState::Executing {
            request_id: request.request_id.clone(),
            started_at: Instant::now(),
        };

        // Bootstrap on first use (load Web APIs polyfills if needed)
        if !isolate.bootstrapped {
            let bootstrap_start = Instant::now();

            if let Err(err) = isolate.runtime.execute_script(
                "bootstrap.js",
                ModuleCodeString::from(crate::bootstrap::bootstrap_source()),
            ) {
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                return (
                    ExecutionOutcome::Err(format!("Bootstrap failed: {}", err)),
                    ExecutionProfile::empty(),
                );
            }

            isolate.bootstrapped = true;
            tracing::debug!(
                "Worker {} bootstrapped {} in {:?}",
                self.worker_id,
                key.name,
                bootstrap_start.elapsed()
            );
        }

        if handler_is_unsupported_script(&key.name) {
            isolate.active_requests = 0;
            isolate.state = IsolateState::Idle;
            return (
                ExecutionOutcome::Err(
                    "TypeScript handlers are not supported; emit JavaScript (export default { fetch }) or serve .ds via dsc.".to_string(),
                ),
                ExecutionProfile::empty(),
            );
        }

        // Enforce the resolved dynamic-code policy before any inline user
        // handler source reaches V8. Validate once per warm isolate rather
        // than on every request; a source hash change creates a new isolate.
        //
        // Scope: this covers the *platform* path, whose tenant bundles arrive
        // as inline source from `PlatformState::resolve_handler`. `deka run`
        // and `deka serve` are ESM and set `handler_code` to the empty string
        // (runtime/src/run.rs, runtime/src/serve.rs), so the guard below is
        // false for them; those are gated by the module-graph validator in
        // #425. The multi-tenant storefronts are the case this protects.
        if !request.request_data.handler_code.trim().is_empty() && !isolate.dynamic_code_validated {
            if let Err(err) = validation::validate_dynamic_code_from_security_context(
                &request.request_data.handler_code,
                &key.name,
            ) {
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                return (ExecutionOutcome::Err(err), ExecutionProfile::empty());
            }
            isolate.dynamic_code_validated = true;
        }

        let use_esm = request.request_data.handler_entry.is_some() && self.config.use_esm;

        // Check if the handler code is already a self-contained async IIFE
        // produced by dsc's bundle stage (e.g. `(async function() { ... })()`).
        // These bundles set globalThis.app internally — double-wrapping them
        // in another sync IIFE breaks top-level await and prevents the async
        // code from resolving before __dekaExecuteRequest checks globalThis.app.
        let is_pre_bundled_iife = {
            let trimmed = request.request_data.handler_code.trim_start();
            trimmed.starts_with("(async function()") || trimmed.starts_with("(function()")
        };

        let wrapped_handler_code = if !use_esm {
            if is_pre_bundled_iife {
                // Pre-bundled IIFE: run directly without re-wrapping.
                // dsc's bundle stage already stripped exports and wrapped in
                // an async IIFE that sets globalThis.app.
                let setup_code =
                    "globalThis.app = undefined; globalThis.Deka = globalThis.Deka || {};";
                if let Err(err) = isolate
                    .runtime
                    .execute_script("setup.js", ModuleCodeString::from(setup_code.to_string()))
                {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }

                // Execute the pre-bundled handler with host dispatchers closed over.
                let handler_result = isolate.runtime.execute_script(
                    "handler.js",
                    ModuleCodeString::from(wrap_with_host_bindings(
                        &request.request_data.handler_code,
                    )),
                );
                match handler_result {
                    Ok(_value) => {
                        // The async IIFE returns a Promise. Run the event
                        // loop unconditionally so globalThis.app gets set
                        // before __dekaExecuteRequest checks it.
                        if let Err(err) = isolate
                            .runtime
                            .run_event_loop(deno_core::PollEventLoopOptions::default())
                            .await
                        {
                            isolate.active_requests = 0;
                            isolate.state = IsolateState::Idle;
                            return (
                                ExecutionOutcome::Err(format!(
                                    "Handler async init failed: {}",
                                    err
                                )),
                                ExecutionProfile::empty(),
                            );
                        }
                    }
                    Err(err) => {
                        let raw = err.to_string();
                        if parse_exit_code(&raw).is_none() {
                            isolate.active_requests = 0;
                            isolate.state = IsolateState::Idle;
                            let formatted = validation::format_runtime_syntax_error(
                                &raw,
                                &request.request_data.handler_code,
                                &key.name,
                            );
                            return (
                                ExecutionOutcome::Err(formatted.unwrap_or_else(|| {
                                    format!("Handler execution failed: {}", err)
                                })),
                                ExecutionProfile::empty(),
                            );
                        }
                    }
                }

                None // Already loaded — skip the wrapped-handler path below
            } else {
                // Execute the handler - transform import/export statements
                // Replace ES6 import with global access
                let handler_code = request
                    .request_data
                    .handler_code
                    .replace(
                        "import { Router, cors, logger, prettyJSON } from 'deka/router'",
                        "const { Router, cors, logger, prettyJSON } = globalThis.__dekaRouter;",
                    )
                    .replace(
                        "import { Router, cors, logger, prettyJSON } from \"deka/router\"",
                        "const { Router, cors, logger, prettyJSON } = globalThis.__dekaRouter;",
                    )
                    .replace(
                        "import { Router } from 'deka/router'",
                        "const { Router } = globalThis.__dekaRouter;",
                    )
                    .replace(
                        "import { Router } from \"deka/router\"",
                        "const { Router } = globalThis.__dekaRouter;",
                    )
                    .replace(
                        "import { Database, Statement } from 'deka/sqlite'",
                        "const { Database, Statement } = globalThis.__dekaSqlite;",
                    )
                    .replace(
                        "import { Database, Statement } from \"deka/sqlite\"",
                        "const { Database, Statement } = globalThis.__dekaSqlite;",
                    )
                    .replace(
                        "import { Database } from 'deka/sqlite'",
                        "const { Database } = globalThis.__dekaSqlite;",
                    )
                    .replace(
                        "import { Database } from \"deka/sqlite\"",
                        "const { Database } = globalThis.__dekaSqlite;",
                    )
                    .replace(
                        "import { t4, T4Client, T4File, write } from 'deka/t4'",
                        "const { t4, T4Client, T4File, write } = globalThis.__dekaT4;",
                    )
                    .replace(
                        "import { t4, T4Client, T4File, write } from \"deka/t4\"",
                        "const { t4, T4Client, T4File, write } = globalThis.__dekaT4;",
                    )
                    .replace(
                        "import { t4 } from 'deka/t4'",
                        "const { t4 } = globalThis.__dekaT4;",
                    )
                    .replace(
                        "import { t4 } from \"deka/t4\"",
                        "const { t4 } = globalThis.__dekaT4;",
                    )
                    .replace(
                        "import { Mesh, IsolatePool, Isolate, serve } from 'deka'",
                        "const { Mesh, IsolatePool, Isolate, serve } = globalThis.__deka;",
                    )
                    .replace(
                        "import { Mesh, IsolatePool, Isolate, serve } from \"deka\"",
                        "const { Mesh, IsolatePool, Isolate, serve } = globalThis.__deka;",
                    )
                    // Remove export default statement - we'll capture 'app' variable directly
                    .replace("export default app", "// export default app")
                    .replace("export default ", "const __dekaDefault = ");

                let wrapped = wrap_with_host_bindings(&format!(
                    "{}\nif (typeof globalThis.app === 'undefined') {{ if (typeof __dekaDefault !== 'undefined') {{ if (typeof __dekaDefault === 'function' && typeof globalThis.__dekaNodeExpressAdapter === 'function' && (typeof __dekaDefault.handle === 'function' || typeof __dekaDefault.listen === 'function')) {{ globalThis.app = globalThis.__dekaNodeExpressAdapter(__dekaDefault); }} else if (__dekaDefault && typeof __dekaDefault === 'object' && typeof __dekaDefault.fetch === 'function') {{ globalThis.app = __dekaDefault; }} else if (__dekaDefault && typeof __dekaDefault === 'object' && !__dekaDefault.__dekaServer && typeof __dekaDefault.routes === 'object' && globalThis.__deka && typeof globalThis.__deka.serve === 'function') {{ globalThis.app = globalThis.__deka.serve(__dekaDefault); }} else {{ globalThis.app = __dekaDefault; }} }} else if (typeof app !== 'undefined') {{ if (typeof app === 'function' && typeof globalThis.__dekaNodeExpressAdapter === 'function' && (typeof app.handle === 'function' || typeof app.listen === 'function')) {{ globalThis.app = globalThis.__dekaNodeExpressAdapter(app); }} else {{ globalThis.app = app; }} }} }}",
                    handler_code
                ));

                let setup_code =
                    "globalThis.app = undefined; globalThis.Deka = globalThis.Deka || {};";
                if let Err(err) = isolate
                    .runtime
                    .execute_script("setup.js", ModuleCodeString::from(setup_code.to_string()))
                {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }

                Some(wrapped)
            }
        } else {
            None
        };

        let tenant_info = Option::<crate::tenant::TenantInfo>::None;
        let shop_secrets = if let Some(info) = tenant_info.as_ref() {
            if info.shop_id.is_empty() {
                HashMap::new()
            } else {
                match secrets_cache.get_secrets_for_shop(&info.shop_id).await {
                    Ok(secrets) => secrets,
                    Err(err) => {
                        tracing::warn!(
                            shop_id = %info.shop_id,
                            error = %err,
                            "failed to fetch shop secrets from harar"
                        );
                        HashMap::new()
                    }
                }
            }
        } else {
            HashMap::new()
        };

        if request.request_data.mode != ExecutionMode::Build {
            if let Err(err) = set_request_globals(
                &mut isolate.runtime,
                &request.request_data.request_value,
                request.request_data.request_parts.as_ref(),
                &self.deka_args,
                request.request_data.handler_entry.as_deref(),
                request.request_data.module_root.as_deref(),
                tenant_info.as_ref(),
                &shop_secrets,
            ) {
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                return (
                    ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                    ExecutionProfile::empty(),
                );
            }
        }

        let exec_mode = match request.request_data.mode {
            ExecutionMode::Module => "module",
            ExecutionMode::StaticRender => "static-render",
            ExecutionMode::Build => "build",
            _ => "request",
        };
        if let Err(err) = isolate.runtime.execute_script(
            "exec_mode.js",
            ModuleCodeString::from(format!(
                "globalThis.__dekaExecMode = {};",
                serde_json::to_string(exec_mode).unwrap_or_else(|_| "\"request\"".to_string())
            )),
        ) {
            isolate.active_requests = 0;
            isolate.state = IsolateState::Idle;
            return (
                ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                ExecutionProfile::empty(),
            );
        }

        if let Some(wrapped_handler_code) = wrapped_handler_code.as_ref() {
            if use_code_cache {
                let source_hash = Self::hash_source(&request.request_data.handler_code);
                if let Err(err) = Self::compile_handler(
                    &mut isolate.runtime,
                    code_cache,
                    source_hash,
                    wrapped_handler_code,
                ) {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    let formatted = validation::format_runtime_syntax_error(
                        &err,
                        &request.request_data.handler_code,
                        &key.name,
                    );
                    return (
                        ExecutionOutcome::Err(formatted.unwrap_or(err)),
                        ExecutionProfile::empty(),
                    );
                }
            } else if let Err(err) = isolate.runtime.execute_script(
                "handler.js",
                ModuleCodeString::from(wrapped_handler_code.to_string()),
            ) {
                let raw = err.to_string();
                if parse_exit_code(&raw).is_none() {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    let formatted = validation::format_runtime_syntax_error(
                        &raw,
                        &request.request_data.handler_code,
                        &key.name,
                    );
                    return (
                        ExecutionOutcome::Err(
                            formatted
                                .unwrap_or_else(|| format!("Handler execution failed: {}", err)),
                        ),
                        ExecutionProfile::empty(),
                    );
                }
            }
        } else if use_esm {
            if !isolate.handler_loaded {
                let spec = match isolate.entry_specifier.as_ref() {
                    Some(spec) => spec,
                    None => {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Err("missing module entry specifier".to_string()),
                            ExecutionProfile::empty(),
                        );
                    }
                };
                let module_id = match isolate.runtime.load_main_es_module(spec).await {
                    Ok(id) => id,
                    Err(err) => {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Err(format!("Failed to load module: {}", err)),
                            ExecutionProfile::empty(),
                        );
                    }
                };
                let eval = isolate.runtime.mod_evaluate(module_id);
                if let Err(err) = isolate
                    .runtime
                    .run_event_loop(deno_core::PollEventLoopOptions::default())
                    .await
                {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Module event loop failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }
                if let Err(err) = eval.await {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Module evaluation failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }
                isolate.handler_loaded = true;
            }
        }

        if request.request_data.mode == ExecutionMode::Module {
            let exit_value = isolate.runtime.execute_script(
                "handler.js",
                ModuleCodeString::from(
                    "const __code = globalThis.__dekaExitCode; globalThis.__dekaExitCode = undefined; __code ?? null".to_string(),
                ),
            );
            if let Ok(value) = exit_value {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &value);
                if let Ok(parsed) = serde_v8::from_v8::<serde_json::Value>(scope, local) {
                    if let Some(code) = parsed.as_i64() {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code })),
                            ExecutionProfile::empty(),
                        );
                    }
                }
            }
        }

        if !isolate.handler_loaded {
            isolate.handler_loaded = true;
            if self.config.debug {
                deka_stdio::log(
                    "handler",
                    &format!("loaded {} on worker {}", key.name, self.worker_id),
                );
            }
        }

        let heap_before_bytes = isolate
            .runtime
            .v8_isolate()
            .get_heap_statistics()
            .used_heap_size();

        // Track CPU time for this execution
        let cpu_start = get_thread_cpu_time();
        let timeout_ms = self.config.request_timeout_ms;
        let timeout_flag = Arc::new(AtomicUsize::new(0));
        let timeout_flag_handle = Arc::clone(&timeout_flag);
        let isolate_handle = isolate.runtime.v8_isolate().thread_safe_handle();

        let watchdog = if timeout_ms > 0 {
            Some(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
                timeout_flag_handle.store(1, Ordering::Relaxed);
                isolate_handle.terminate_execution();
            }))
        } else {
            None
        };

        let exec_start = Instant::now();
        let mut needs_event_loop = false;
        let result = if request.request_data.mode == ExecutionMode::Module {
            isolate
                .runtime
                .execute_script(
                    "handler.js",
                    ModuleCodeString::from("undefined".to_string()),
                )
                .map_err(|err| err.to_string())
        } else if request.request_data.mode == ExecutionMode::StaticRender {
            isolate
                .runtime
                .execute_script(
                    "handler.js",
                    ModuleCodeString::from("globalThis.__dekaStaticRender()".to_string()),
                )
                .map_err(|err| err.to_string())
        } else if request.request_data.mode == ExecutionMode::Build {
            isolate
                .runtime
                .execute_script(
                    "handler.js",
                    ModuleCodeString::from(
                        "(async () => { if (typeof globalThis.__dekaBuild !== 'function') { throw new Error('build entry must export a default async function'); } const value = await globalThis.__dekaBuild(); return JSON.stringify(value, (_key, item) => item instanceof Uint8Array ? { __deka_bytes: Array.from(item) } : item); })()".to_string(),
                    ),
                )
                .map_err(|err| err.to_string())
        } else {
            // Execute the handler fetch using the globals
            const EXEC_CALL: &str = r#"globalThis.__dekaExecuteRequest()"#;
            let code = EXEC_CALL.to_string();
            isolate
                .runtime
                .execute_script("handler.js", ModuleCodeString::from(code))
                .map_err(|err| err.to_string())
        };

        let result = match result {
            Ok(value) => value,
            Err(err) => {
                if let Some(watchdog) = watchdog {
                    watchdog.abort();
                }
                if let Some(code) = parse_exit_code(&err) {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code })),
                        ExecutionProfile::empty(),
                    );
                }
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                let profile = finalize_profile(
                    heap_before_bytes,
                    isolate,
                    exec_start.elapsed().as_millis() as u64,
                    0,
                    0,
                );
                return (
                    ExecutionOutcome::Err(format!("Handler execution failed: {}", err)),
                    profile,
                );
            }
        };
        let exec_script_ms = exec_start.elapsed().as_millis() as u64;

        if matches!(
            request.request_data.mode,
            ExecutionMode::Request
                | ExecutionMode::StaticRender
                | ExecutionMode::Build
                | ExecutionMode::Module
        ) {
            // Run event loop to complete async operations
            {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &result);
                if let Ok(promise) = deno_core::v8::Local::<deno_core::v8::Promise>::try_from(local)
                {
                    match promise.state() {
                        deno_core::v8::PromiseState::Pending => {
                            needs_event_loop = true;
                        }
                        _ => needs_event_loop = false,
                    }
                }
            }
        }

        let event_loop_ms = if needs_event_loop {
            let event_start = Instant::now();
            if let Err(err) = isolate
                .runtime
                .run_event_loop(deno_core::PollEventLoopOptions::default())
                .await
            {
                if let Some(watchdog) = watchdog {
                    watchdog.abort();
                }
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                let profile = finalize_profile(
                    heap_before_bytes,
                    isolate,
                    exec_script_ms,
                    event_start.elapsed().as_millis() as u64,
                    0,
                );
                return (
                    ExecutionOutcome::Err(format!("Event loop failed: {}", err)),
                    profile,
                );
            }
            event_start.elapsed().as_millis() as u64
        } else {
            0
        };

        if let Some(watchdog) = watchdog {
            watchdog.abort();
        }

        // Calculate CPU time consumed
        let cpu_elapsed = get_thread_cpu_time() - cpu_start;
        isolate.total_cpu_time += cpu_elapsed;

        update_heap_stats(isolate);

        // Get the result from the promise
        let decode_start = Instant::now();
        let outcome = if request.request_data.mode == ExecutionMode::Module {
            let exit_value = isolate.runtime.execute_script(
                "handler.js",
                ModuleCodeString::from(
                    "const __code = globalThis.__dekaExitCode; globalThis.__dekaExitCode = undefined; __code ?? null".to_string(),
                ),
            );
            if let Ok(value) = exit_value {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &value);
                if let Ok(parsed) = serde_v8::from_v8::<serde_json::Value>(scope, local) {
                    if let Some(code) = parsed.as_i64() {
                        ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code }))
                    } else {
                        ExecutionOutcome::Ok(serde_json::Value::Null)
                    }
                } else {
                    ExecutionOutcome::Ok(serde_json::Value::Null)
                }
            } else {
                ExecutionOutcome::Ok(serde_json::Value::Null)
            }
        } else {
            deno_core::scope!(scope, &mut isolate.runtime);
            let local = deno_core::v8::Local::new(scope, &result);

            let value_result: Result<deno_core::v8::Local<deno_core::v8::Value>, String> =
                if let Ok(promise) = deno_core::v8::Local::<deno_core::v8::Promise>::try_from(local)
                {
                    match promise.state() {
                        deno_core::v8::PromiseState::Fulfilled => Ok(promise.result(scope)),
                        deno_core::v8::PromiseState::Rejected => {
                            let reason = promise.result(scope);
                            Err(format!(
                                "Handler rejected: {}",
                                reason.to_rust_string_lossy(scope)
                            ))
                        }
                        deno_core::v8::PromiseState::Pending => {
                            Err("Handler promise still pending after event loop".to_string())
                        }
                    }
                } else {
                    Ok(local)
                };

            match value_result {
                Ok(value) => match serde_v8::from_v8::<serde_json::Value>(scope, value) {
                    Ok(value) => ExecutionOutcome::Ok(value),
                    Err(err) => ExecutionOutcome::Err(format!(
                        "Handler returned non-serializable result: {}",
                        err
                    )),
                },
                Err(err) => ExecutionOutcome::Err(err),
            }
        };

        isolate.active_requests = 0;
        isolate.state = IsolateState::Idle;
        let outcome = if timeout_flag.load(Ordering::Relaxed) == 1 {
            isolate.state = IsolateState::Stuck {
                request_id: request.request_id.clone(),
                started_at: Instant::now(),
                timeout_triggered: true,
            };
            ExecutionOutcome::TimedOut
        } else {
            outcome
        };

        let result_decode_ms = decode_start.elapsed().as_millis() as u64;
        let profile = finalize_profile(
            heap_before_bytes,
            isolate,
            exec_script_ms,
            event_loop_ms,
            result_decode_ms,
        );
        (outcome, profile)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ExecutionMode, ExecutionSecurity, HandlerKey, IsolatePool, PoolConfig, RequestData,
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn listener_close_and_deadline_through_host_ops() {
        let pool = IsolatePool::new(
            PoolConfig {
                num_workers: 1,
                enable_code_cache: false,
                ..PoolConfig::default()
            },
            Arc::new(platform_server::extensions_for_php_server),
        );
        let response = pool.execute(
            HandlerKey::new("listener_close_and_deadline_through_host_ops"),
            RequestData {
                handler_code: r#"
                    globalThis.app = function() {
                        const listen = () => __deka_host('net', 'listen', ['127.0.0.1', 0], ['net']);
                        const first = listen();
                        const close = __deka_host('net', 'close', [first.handle], ['net']);
                        const second = listen();
                        const deadline = __deka_host('net', 'set_deadline', [second.handle, 0], ['net']);
                        const cleanup = __deka_host('net', 'close', [second.handle], ['net']);
                        const stale = __deka_host('net', 'set_deadline', [second.handle, 0], ['net']);
                        return { status: 200, headers: {}, body: JSON.stringify({first, close, second, deadline, cleanup, stale}) };
                    };
                "#.to_string(),
                handler_entry: None,
                module_root: None,
                request_value: serde_json::Value::Null,
                request_parts: None,
                mode: ExecutionMode::Request,
                security: ExecutionSecurity {
                    policy_json: r#"{"security":{"allow":{"net":["127.0.0.1:0"]},"prompt":false}}"#.to_string(),
                    no_prompt: true,
                },
            },
        ).await.expect("pool execution");
        assert!(
            response.success,
            "host exception escaped: {:?}",
            response.error
        );
        let result = response.result.expect("response result");
        let body: serde_json::Value =
            serde_json::from_str(result["body"].as_str().unwrap()).unwrap();
        for action in ["first", "close", "second", "deadline", "cleanup"] {
            assert_eq!(body[action]["ok"], true, "{action}: {body}");
        }
        assert_ne!(body["first"]["handle"], body["second"]["handle"]);
        assert_eq!(body["stale"]["ok"], false, "{body}");
    }

    // Run the production bootstrap and real Rust host ops. Assert in Rust after
    // execution: a JS throw must fail the test, not skip an unreachable check.
    #[tokio::test]
    async fn thrown_host_ops_return_error_envelopes() {
        let pool = IsolatePool::new(
            PoolConfig {
                num_workers: 1,
                enable_code_cache: false,
                ..PoolConfig::default()
            },
            Arc::new(platform_server::extensions_for_php_server),
        );
        let response = pool.execute(
            HandlerKey::new("thrown_host_ops_return_error_envelopes"),
            RequestData {
                handler_code: r#"
                    globalThis.app = function() {
                        const accept = __deka_host('net', 'accept', [0], ['net']);
                        const upgrade = __deka_host('tls', 'upgrade', [1, ''], ['tls']);
                        // A synchronous fs permission throw must retain #758's
                        // typed error, not fall back to the string envelope.
                        const fs = __deka_host('fs', 'read_file_sync', ['/deka-envelope-denied'], ['fs']);
                        return { status: 200, headers: {}, body: JSON.stringify({accept, upgrade, fs}) };
                    };
                "#.to_string(),
                handler_entry: None,
                module_root: None,
                request_value: serde_json::Value::Null,
                request_parts: None,
                mode: ExecutionMode::Request,
                security: ExecutionSecurity {
                    policy_json: r#"{"security":{"allow":{},"prompt":false}}"#.to_string(),
                    no_prompt: true,
                },
            },
        ).await.expect("pool execution");
        assert!(
            response.success,
            "host exception escaped: {:?}",
            response.error
        );
        let result = response.result.expect("response result");
        let body: serde_json::Value =
            serde_json::from_str(result["body"].as_str().expect("response body"))
                .expect("JSON envelopes");
        assert_eq!(body["accept"]["ok"], false);
        assert!(
            body["accept"]["error"]
                .as_str()
                .expect("accept error")
                .contains("unknown listener handle"),
            "{body}"
        );
        assert_eq!(body["upgrade"]["ok"], false);
        assert!(
            body["upgrade"]["error"]
                .as_str()
                .expect("upgrade error")
                .contains("unknown handle 1"),
            "{body}"
        );
        assert_eq!(body["fs"]["ok"], false);
        assert_eq!(body["fs"]["error"]["__enum"], "FsError");
        assert_eq!(body["fs"]["error"]["__case"], "PermissionDenied");
    }
}
