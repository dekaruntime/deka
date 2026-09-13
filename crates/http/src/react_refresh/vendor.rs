//! Frozen React / react-refresh development builds served by `deka dev`.
//!
//! Bytes live in `crates/http/vendor/react/` with locked hashes and licenses.
//! Production `@js/react*` builtins (rfd#64 amendment 2) live in
//! `crates/pool/vendor/react-prod/` and are always-on; this tree stays
//! feature-gated. The served URLs (`/_deka/react/...`) stay stable.

pub struct VendorFile {
    pub body: &'static str,
}

pub fn get(name: &str) -> Option<VendorFile> {
    let body = match name {
        "react.js" => include_str!("../../vendor/react/react.js"),
        "scheduler.js" => include_str!("../../vendor/react/scheduler.js"),
        "react-dom.js" => include_str!("../../vendor/react/react-dom.js"),
        "react-dom-client.js" => include_str!("../../vendor/react/react-dom-client.js"),
        "jsx-runtime.js" => include_str!("../../vendor/react/jsx-runtime.js"),
        "jsx-dev-runtime.js" => include_str!("../../vendor/react/jsx-dev-runtime.js"),
        "refresh-runtime.js" => include_str!("../../vendor/react/refresh-runtime.js"),
        "HASHES" => include_str!("../../vendor/react/HASHES"),
        "NOTICE" => include_str!("../../vendor/react/NOTICE"),
        _ => return None,
    };
    Some(VendorFile { body })
}

pub fn content_type(name: &str) -> &'static str {
    if name.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    }
}

#[cfg(test)]
mod tests {
    use super::get;

    #[test]
    fn development_react_exports_refresh_hook_surface() {
        let react = get("react.js").expect("react.js");
        assert!(react.body.contains("19.1.1"));
        assert!(react.body.contains("useState"));
        let refresh = get("refresh-runtime.js").expect("refresh-runtime.js");
        assert!(refresh.body.contains("injectIntoGlobalHook"));
        assert!(refresh.body.contains("performReactRefresh"));
        assert!(refresh.body.contains("exports.register"));
        let jsx_dev = get("jsx-dev-runtime.js").expect("jsx-dev-runtime.js");
        assert!(jsx_dev.body.contains("jsxDEV"));
        assert!(!jsx_dev.body.contains("e.jsxDEV=void 0"));
    }

    #[test]
    fn node_refresh_runtime_contract() {
        let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("vendor/react/runtime-contract.mjs");
        let output = std::process::Command::new("node")
            .arg(&script)
            .output()
            .expect("run node");
        assert!(
            output.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn hashes_lock_upstream_cjs() {
        let hashes = get("HASHES").expect("HASHES").body;
        assert!(hashes.contains("react@19.1.1"));
        assert!(hashes.contains("react-refresh@0.17.0"));
        assert!(hashes.contains("c3140127dd572acd866efa06e63c5c27f79daa5879e4c39af5c91644f8574751"));
    }
}
