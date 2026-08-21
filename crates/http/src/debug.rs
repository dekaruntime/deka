pub(crate) fn http_debug_enabled() -> bool {
    std::env::var("DEKA_HTTP_DEBUG").is_ok()
}
