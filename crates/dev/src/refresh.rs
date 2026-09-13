//! Watch-side Fast Refresh: recompile changed client modules and push a
//! `js-update` over the existing HMR websocket.

pub(crate) fn push_js_update(changed: &[String]) -> bool {
    deka_http::react_refresh::broadcast_changed(changed)
}
