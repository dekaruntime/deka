fn main() {
    let backend =
        deka_native_bench::quickjs::QuickJsBackend::new().expect("initialize QuickJS backend");
    deka_native_ui::run(deka_native_bench::App::new(backend));
}
