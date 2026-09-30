fn main() {
    #[cfg(feature = "v8-backend")]
    let backend = deka_native_bench::v8::V8Backend::new();
    #[cfg(not(feature = "v8-backend"))]
    let backend = deka_native_bench::RustBackend;
    deka_native_ui::run(deka_native_bench::App::new(backend));
}
