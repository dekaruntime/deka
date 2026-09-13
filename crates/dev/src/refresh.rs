//! Watch-side Fast Refresh: recompile changed client modules and push a
//! `js-update` over the existing HMR websocket.

pub(crate) fn push_js_update(changed: &[String]) -> bool {
    #[cfg(feature = "dev-server")]
    {
        deka_http::react_refresh::broadcast_changed(changed)
    }
    #[cfg(not(feature = "dev-server"))]
    {
        let _ = changed;
        false
    }
}
