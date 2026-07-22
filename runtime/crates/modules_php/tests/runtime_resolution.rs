use deno_core::{JsRuntime, ModuleCodeString, ModuleSpecifier, RuntimeOptions};

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: String) -> Self {
        let previous = std::env::var(key).ok();
        // Tests run on a current-thread Tokio runtime; the process env is
        // still globally shared in Rust 2024, so keep the mutation scoped.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn deka_php_runtime_resolves_local_unscoped_before_global_scoped() {
    let local = tempfile::tempdir().expect("local tempdir");
    let global = tempfile::tempdir().expect("global tempdir");
    let local_root = local.path();
    let global_root = global.path();

    std::fs::create_dir_all(local_root.join("php_modules/http")).expect("local http dir");
    std::fs::create_dir_all(global_root.join("php_modules/@deka/http"))
        .expect("global scoped http dir");
    std::fs::write(
        local_root.join("php_modules/http/index.phpx"),
        "export function http_get($url: string): object { return ['tier' => 'local']; }\n",
    )
    .expect("local http module");
    std::fs::write(
        global_root.join("php_modules/@deka/http/index.phpx"),
        "export function http_get($url: string): object { return ['tier' => 'global']; }\n",
    )
    .expect("global http module");

    std::fs::write(
        local_root.join("deka.lock"),
        serde_json::json!({
            "lockfileVersion": 1,
            "php": {
                "cache": {
                    "modules": {
                        "http": {
                            "src": "php_modules/http/index.phpx"
                        }
                    }
                }
            }
        })
        .to_string(),
    )
    .expect("local lock");
    std::fs::write(
        global_root.join("deka.lock"),
        serde_json::json!({
            "lockfileVersion": 1,
            "php": {
                "cache": {
                    "modules": {
                        "@deka/http": {
                            "src": "php_modules/@deka/http/index.phpx"
                        }
                    }
                }
            }
        })
        .to_string(),
    )
    .expect("global lock");

    let entry_path = local_root.join("app/main.phpx");
    std::fs::create_dir_all(entry_path.parent().expect("entry parent")).expect("entry dir");
    std::fs::write(&entry_path, "import { http_get } from '@deka/http'\n").expect("entry");
    let policy = serde_json::json!({
        "security": {
            "allow": {
                "read": [
                    local_root.to_string_lossy(),
                    global_root.to_string_lossy()
                ]
            },
            "prompt": false
        }
    })
    .to_string();
    let _policy_guard = EnvGuard::set("DEKA_SECURITY_POLICY", policy);

    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: modules_php::extensions(),
        ..Default::default()
    });

    let prelude_source = include_str!("../src/modules/deka_php/php.js").replace(
        "export { servePhp as servePhp };",
        "globalThis.__dekaTestResolveImportTarget = resolveImportTarget;\nexport { servePhp as servePhp };",
    );
    let prelude_module = ModuleSpecifier::parse("ext:deka_test/load_deka_php_prelude.js")
        .expect("parse deka PHP prelude test module specifier");
    let prelude_module_id = runtime
        .load_side_es_module_from_code(&prelude_module, ModuleCodeString::from(prelude_source))
        .await
        .expect("load deka PHP prelude test module");
    let prelude_eval = runtime.mod_evaluate(prelude_module_id);
    runtime
        .run_event_loop(deno_core::PollEventLoopOptions::default())
        .await
        .expect("load deka PHP prelude test module");
    prelude_eval
        .await
        .expect("evaluate deka PHP prelude test module");

    let entry_path_js =
        serde_json::to_string(&entry_path.to_string_lossy()).expect("entry path json");
    let modules_root_js = serde_json::to_string(&local_root.join("php_modules").to_string_lossy())
        .expect("modules root json");
    let expected_path_js = serde_json::to_string(
        &local_root
            .join("php_modules/http/index.phpx")
            .to_string_lossy(),
    )
    .expect("expected path json");
    let global_root_js =
        serde_json::to_string(&global_root.to_string_lossy()).expect("global root json");

    let script = format!(
        r#"
        globalThis.process.env.PHPX_MODULE_ROOT = {global_root_js};
        const resolved = globalThis.__dekaTestResolveImportTarget(
          '@deka/http',
          {entry_path_js},
          {modules_root_js}
        );
        if (resolved.filePath !== {expected_path_js}) {{
          throw new Error(`expected local unscoped module, got ${{JSON.stringify(resolved)}}`);
        }}
        if (resolved.moduleId !== 'http') {{
          throw new Error(`expected local module id http, got ${{resolved.moduleId}}`);
        }}
        "#,
    );

    let test_module = ModuleSpecifier::parse("ext:deka_test/runtime_resolution_test.js")
        .expect("parse runtime resolution test module specifier");
    let test_module_id = runtime
        .load_side_es_module_from_code(&test_module, ModuleCodeString::from(script))
        .await
        .expect("load runtime resolution test module");
    let test_eval = runtime.mod_evaluate(test_module_id);
    runtime
        .run_event_loop(deno_core::PollEventLoopOptions::default())
        .await
        .expect("run runtime resolution test module");
    test_eval
        .await
        .expect("evaluate runtime resolution test module");
}
