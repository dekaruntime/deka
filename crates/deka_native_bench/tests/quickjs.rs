#![cfg(feature = "quickjs-backend")]

use deka_native_bench::{App, quickjs::QuickJsBackend};
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, Context, Exception, Function, Module, Promise,
    Runtime, TypedArray,
    function::Async,
    loader::{BuiltinLoader, BuiltinResolver},
};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

fn compiled_counter() -> String {
    deka_compile::compile_to_js(include_str!("fixtures/counter.ds"), "counter.ds")
        .expect("real Deka fixture must compile")
        .js
}

#[test]
fn native_counter_executes_quickjs_events() {
    let app = App::new(QuickJsBackend::new().unwrap());
    assert_eq!(
        deka_native_ui::exercise(app, 3),
        "Deka native backend comparison Count: 3"
    );
}

#[test]
fn compiler_output_imports_and_closures_execute() {
    let runtime = Runtime::new().unwrap();
    runtime.set_loader(
        BuiltinResolver::default().with_module("counter.js"),
        BuiltinLoader::default().with_module("counter.js", compiled_counter()),
    );
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let module = Module::declare(
            ctx.clone(),
            "entry.js",
            r#"
            import {increment, label} from 'counter.js';
            export const result = label(increment(41));
        "#,
        )
        .catch(&ctx)
        .unwrap();
        let (module, ready) = module.eval().catch(&ctx).unwrap();
        ready.finish::<()>().catch(&ctx).unwrap();
        assert_eq!(module.get::<_, String>("result").unwrap(), "Count: 42");
        // An absent module must fail; the loader does not silently fall through.
        assert!(Module::evaluate(ctx.clone(), "missing.js", "import 'absent';").is_err());
        let _ = ctx.catch();
    });
}

#[tokio::test(flavor = "current_thread")]
async fn compiled_async_code_awaits_rust_future_and_propagates_rejection() {
    let runtime = AsyncRuntime::new().unwrap();
    let context = AsyncContext::full(&runtime).await.unwrap();
    let completed = Rc::new(Cell::new(false));
    let observed = completed.clone();
    tokio::time::timeout(
        Duration::from_secs(3),
        context.async_with(async |ctx| {
            let delay = Function::new(
                ctx.clone(),
                Async(move |value: i32| {
                    let completed = completed.clone();
                    async move {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        completed.set(true);
                        Ok::<_, rquickjs::Error>(value + 1)
                    }
                }),
            )
            .unwrap();
            ctx.globals().set("hostIncrement", delay).unwrap();
            let (module, ready) = Module::declare(ctx.clone(), "counter.js", compiled_counter())
                .catch(&ctx)
                .unwrap()
                .eval()
                .catch(&ctx)
                .unwrap();
            ready.into_future::<()>().await.catch(&ctx).unwrap();
            let later: Function = module.get("later").unwrap();
            ctx.globals().set("later", later).unwrap();
            let pending: Promise = ctx
                .eval("(async () => later(await hostIncrement(40)))()")
                .catch(&ctx)
                .unwrap();
            assert!(!observed.get(), "host future must actually suspend");
            assert_eq!(pending.into_future::<i32>().await.catch(&ctx).unwrap(), 42);
            assert!(observed.get());
            let rejected: Promise = ctx
                .eval("Promise.reject(new Error('probe rejection'))")
                .unwrap();
            let error = rejected.into_future::<()>().await.catch(&ctx).unwrap_err();
            assert!(error.to_string().contains("probe rejection"), "{error}");
        }),
    )
    .await
    .expect("async bridge must make progress");
}

#[test]
fn rust_callback_transfers_bytes_and_errors() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let callback = Function::new(ctx.clone(), move |bytes: TypedArray<'_, u8>| {
            calls.set(calls.get() + 1);
            (0..bytes.len())
                .map(|index| bytes.as_object().get::<_, u8>(index.to_string()).map(u32::from))
                .sum::<rquickjs::Result<u32>>()
        }).unwrap();
        ctx.globals().set("sumBytes", callback).unwrap();
        assert_eq!(ctx.eval::<u32, _>("sumBytes(new Uint8Array([1, 2, 255]))").unwrap(), 258);
        assert_eq!(observed.get(), 1);
        let error = Function::new(ctx.clone(), |ctx: rquickjs::Ctx<'_>| -> rquickjs::Result<()> {
            Err(Exception::throw_type(&ctx, "probe denied"))
        }).unwrap();
        ctx.globals().set("denied", error).unwrap();
        assert!(ctx.eval::<bool, _>("(() => { try { denied(); return false; } catch (e) { return e instanceof TypeError && e.message === 'probe denied'; } })()").unwrap());
    });
}

#[test]
fn collector_reclaims_cycles_and_interrupt_stops_runaway_code() {
    let runtime = Runtime::new().unwrap();
    runtime.set_memory_limit(16 * 1024 * 1024);
    let context = Context::full(&runtime).unwrap();
    runtime.run_gc();
    let before = runtime.memory_usage().obj_count;
    context.with(|ctx| {
        ctx.eval::<(), _>(
            r#"
        globalThis.held = Array.from({length: 1000}, () => { const x = {}; x.self = x; return x; });
    "#,
        )
        .unwrap()
    });
    assert!(runtime.memory_usage().obj_count >= before + 1000);
    context.with(|ctx| ctx.eval::<(), _>("globalThis.held = null").unwrap());
    runtime.run_gc();
    assert!(runtime.memory_usage().obj_count < before + 20);

    let deadline = Instant::now() + Duration::from_millis(25);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    context.with(|ctx| {
        let error = ctx
            .eval::<(), _>("while (true) {}")
            .catch(&ctx)
            .unwrap_err();
        assert!(error.to_string().contains("interrupted"), "{error}");
    });
    runtime.set_interrupt_handler(None);
    context.with(|ctx| assert_eq!(ctx.eval::<i32, _>("6 * 7").unwrap(), 42));
}

#[test]
fn compiled_dsx_uses_existing_jsx_factory_and_retains_click_closure() {
    let source =
        deka_compile::compile_to_js(include_str!("fixtures/component.dsx"), "component.dsx")
            .expect("DSX must compile")
            .js;
    // Execute the repository's actual vendored factory, not a JSX mock.
    let jsx = format!(
        "const exports = {{}};\n{}\nexport const jsx = exports.jsx, jsxs = exports.jsxs, Fragment = exports.Fragment;",
        include_str!("../../pool/vendor/react-prod/cjs/react-jsx-runtime.production.js"),
    );
    let react = format!(
        "const exports = {{}};\n{}\nexport default exports;",
        include_str!("../../pool/vendor/react-prod/cjs/react.production.js"),
    );
    let runtime = Runtime::new().unwrap();
    runtime.set_loader(
        BuiltinResolver::default()
            .with_module("@js/react/jsx-runtime")
            .with_module("@js/react"),
        BuiltinLoader::default()
            .with_module("@js/react/jsx-runtime", jsx)
            .with_module("@js/react", react),
    );
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let (module, ready) = Module::declare(ctx.clone(), "component.js", source)
            .catch(&ctx)
            .unwrap()
            .eval()
            .catch(&ctx)
            .unwrap();
        ready.finish::<()>().catch(&ctx).unwrap();
        ctx.globals()
            .set("Counter", module.get::<_, Function>("Counter").unwrap())
            .unwrap();
        assert_eq!(
            ctx.eval::<String, _>("Counter().props.children").unwrap(),
            "0"
        );
        ctx.eval::<(), _>("globalThis.button = Counter(); button.props.onClick();")
            .unwrap();
        // A collection between calls must not destroy a retained event closure.
    });
    runtime.run_gc();
    context.with(|ctx| {
        ctx.eval::<(), _>("button.props.onClick()").unwrap();
        assert_eq!(
            ctx.eval::<String, _>("Counter().props.children").unwrap(),
            "2"
        );
        assert_eq!(ctx.eval::<String, _>("Counter().type").unwrap(), "button");
    });
}
