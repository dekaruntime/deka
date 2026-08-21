use deno_core::{JsRuntime, ModuleCodeString, ModuleSpecifier, RuntimeOptions};

#[tokio::test(flavor = "current_thread")]
async fn crypto_bcrypt_verify_bridge_returns_bool_result() {
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![modules_php::modules::php::init()],
        ..Default::default()
    });

    let prelude_source = include_str!("../src/modules/deka_php/php.js").replace(
        "export { servePhp as servePhp };",
        "globalThis.__dekaTestRouteHostCall = routeHostCall;",
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

    let script = r#"
        const routeHostCall = globalThis.__dekaTestRouteHostCall;
        if (typeof routeHostCall !== 'function') {
          throw new Error('routeHostCall was not exposed for the bridge test');
        }
        const hash = "$2b$10$DqpfeHg1RhyMilY/GTQvgeahRja6yf5aL8dYoH6EwABQY.CZ.pnNu";
        const good = Object.fromEntries(routeHostCall("crypto", "bcrypt_verify", {
          password: "password123",
          hash
        }));
        const bad = Object.fromEntries(routeHostCall("crypto", "bcrypt_verify", {
          password: "wrongpassword",
          hash
        }));

        if (good.ok !== true || good.valid !== true) {
          throw new Error(`expected valid bcrypt password, got ${JSON.stringify(good)}`);
        }
        if (bad.ok !== true || bad.valid !== false) {
          throw new Error(`expected invalid bcrypt password, got ${JSON.stringify(bad)}`);
        }
    "#;

    let test_module = ModuleSpecifier::parse("ext:deka_test/crypto_bcrypt_verify_bridge_test.js")
        .expect("parse bcrypt bridge test module specifier");
    let test_module_id = runtime
        .load_side_es_module_from_code(&test_module, ModuleCodeString::from(script.to_string()))
        .await
        .expect("load bcrypt bridge test module");
    let test_eval = runtime.mod_evaluate(test_module_id);
    runtime
        .run_event_loop(deno_core::PollEventLoopOptions::default())
        .await
        .expect("run bcrypt bridge test module");
    test_eval.await.expect("evaluate bcrypt bridge test module");
}
