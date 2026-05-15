use deno_core::{JsRuntime, ModuleCodeString, RuntimeOptions};

#[cfg(unix)]
struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

#[cfg(unix)]
impl EnvGuard {
    fn set(key: &'static str, value: String) -> Self {
        let previous = std::env::var(key).ok();
        // Tests run in a single-threaded Tokio runtime here; setting process
        // env is still unsafe in Rust 2024 because it is globally shared.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

#[cfg(unix)]
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn deka_fs_read_file_sync_rejects_symlink_escape() {
    let tenant_a = tempfile::tempdir().expect("tenant A tempdir");
    let tenant_b = tempfile::tempdir().expect("tenant B tempdir");

    let root_a = tenant_a.path().canonicalize().expect("canonicalize tenant A");
    let root_b = tenant_b.path().canonicalize().expect("canonicalize tenant B");

    let a_secret = root_a.join("secret.txt");
    let b_secret = root_b.join("secret.txt");
    std::fs::write(&a_secret, "A-content").expect("write tenant A secret");
    std::fs::write(&b_secret, "B-content").expect("write tenant B secret");

    let peek_b = root_a.join("peek_b");
    std::os::unix::fs::symlink(&b_secret, &peek_b).expect("create escape symlink");

    let root_a_js = serde_json::to_string(&root_a.to_string_lossy()).expect("json root A");
    let a_secret_js = serde_json::to_string(&a_secret.to_string_lossy()).expect("json A secret");
    let peek_b_js = serde_json::to_string(&peek_b.to_string_lossy()).expect("json symlink");
    let policy = serde_json::json!({
        "security": {
            "allow": {
                "read": [root_a.to_string_lossy()]
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

    runtime
        .execute_script(
            "load_php_extension.js",
            ModuleCodeString::from("import('ext:php_core/php.js');".to_string()),
        )
        .expect("start PHP extension import");
    runtime
        .run_event_loop(deno_core::PollEventLoopOptions::default())
        .await
        .expect("load PHP extension");

    let script = format!(
        r#"
        globalThis.__dekaFsTenantRoot = {root_a_js};

        if (!globalThis.__dekaFs || typeof globalThis.__dekaFs.readFileSync !== "function") {{
          throw new Error("__dekaFs.readFileSync wrapper was not installed");
        }}

        const inside = globalThis.__dekaFs.readFileSync({a_secret_js}, "utf8");
        if (inside !== "A-content") {{
          throw new Error(`expected tenant file read to return A-content, got ${{inside}}`);
        }}

        const escaped = globalThis.__dekaFs.readFileSync({peek_b_js}, "utf8");
        if (escaped !== null) {{
          throw new Error(`expected symlink escape to be rejected, got ${{escaped}}`);
        }}
        "#
    );

    runtime
        .execute_script(
            "symlink_escape_test.js",
            ModuleCodeString::from(script),
        )
        .expect("__dekaFs symlink escape check");
}

#[cfg(not(unix))]
#[test]
fn deka_fs_read_file_sync_rejects_symlink_escape() {
    // The runtime symlink escape regression uses Unix symlinks because the
    // production platform is Unix-like and Windows symlink creation can require
    // elevated privileges.
}
