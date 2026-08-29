use deno_core::{JsRuntime, ModuleCodeString, ModuleSpecifier, RuntimeOptions};

#[tokio::test(flavor = "current_thread")]
async fn crypto_bcrypt_verify_bridge_returns_bool_result() {
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![deka_host::modules::php::init()],
        ..Default::default()
    });

    let prelude_source = include_str!("../src/modules/deka_php/php.js").replace(
        "export { servePhp as servePhp };",
        "globalThis.__dekaTestRouteHostCall = routeHostCall;",
    );
    let prelude_module = ModuleSpecifier::parse("file:///deka_test/load_deka_php_prelude.js")
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

    let test_module = ModuleSpecifier::parse("file:///deka_test/crypto_bcrypt_verify_bridge_test.js")
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

#[tokio::test(flavor = "current_thread")]
async fn crypto_digest_hmac_secure_compare_bridge() {
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![deka_host::modules::php::init()],
        ..Default::default()
    });

    let prelude_source = include_str!("../src/modules/deka_php/php.js").replace(
        "export { servePhp as servePhp };",
        "globalThis.__dekaTestRouteHostCall = routeHostCall;",
    );
    let prelude_module = ModuleSpecifier::parse("file:///deka_test/load_deka_php_prelude_crypto.js")
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
        const digest = Object.fromEntries(routeHostCall("crypto", "digest", {
          algorithm: "sha256",
          data: []
        }));
        if (digest.ok !== true || !Array.isArray(digest.data) || digest.data.length !== 32 || digest.data[0] !== 0xe3) {
          throw new Error(`expected sha256 empty vector, got ${JSON.stringify(digest)}`);
        }
        const mac = Object.fromEntries(routeHostCall("crypto", "hmac", {
          algorithm: "sha256",
          key: [0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b,0x0b],
          data: Array.from(new TextEncoder().encode("Hi There"))
        }));
        if (mac.ok !== true || !Array.isArray(mac.data) || mac.data.length !== 32) {
          throw new Error(`expected hmac sha256, got ${JSON.stringify(mac)}`);
        }
        const same = Object.fromEntries(routeHostCall("crypto", "secure_compare", { a: [1, 2], b: [1, 2] }));
        const diff = Object.fromEntries(routeHostCall("crypto", "secure_compare", { a: [1, 2], b: [1, 3] }));
        if (same.ok !== true || same.data !== true) {
          throw new Error(`expected equal compare, got ${JSON.stringify(same)}`);
        }
        if (diff.ok !== true || diff.data !== false) {
          throw new Error(`expected unequal compare, got ${JSON.stringify(diff)}`);
        }
    "#;

    let test_module = ModuleSpecifier::parse("file:///deka_test/crypto_digest_hmac_bridge_test.js")
        .expect("parse digest bridge test module specifier");
    let test_module_id = runtime
        .load_side_es_module_from_code(&test_module, ModuleCodeString::from(script.to_string()))
        .await
        .expect("load digest bridge test module");
    let test_eval = runtime.mod_evaluate(test_module_id);
    runtime
        .run_event_loop(deno_core::PollEventLoopOptions::default())
        .await
        .expect("run digest bridge test module");
    test_eval.await.expect("evaluate digest bridge test module");
}
