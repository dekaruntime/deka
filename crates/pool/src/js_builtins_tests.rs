use super::*;

#[test]
fn hashes_lock_upstream_cjs() {
    assert!(HASHES.contains(REACT_SHA));
    assert!(HASHES.contains(JSX_RUNTIME_SHA));
    assert!(HASHES.contains(REACT_DOM_SHA));
    assert!(HASHES.contains(REACT_DOM_CLIENT_SHA));
    assert!(HASHES.contains(SCHEDULER_SHA));
    assert!(HASHES.contains(SERVER_LEGACY_SHA));
    assert!(HASHES.contains(SERVER_BROWSER_SHA));
    assert!(
        HASHES.contains(&format!("react@{REACT_VERSION}"))
            || HASHES.contains("react.production.js")
    );
}

#[test]
fn public_specs_are_runtime_provided() {
    for spec in USER_SPECS {
        assert!(is_builtin(spec), "{spec}");
        assert!(file_for_user_spec(spec).is_some(), "{spec}");
    }
    assert!(!is_builtin("@js/lodash"));
    assert!(!is_builtin("@js/react/jsx-dev-runtime"));
    assert_eq!(
        filter_imports(&["@js/react".into(), "@js/chosen".into(), "./local.js".into()]),
        vec!["@js/chosen".to_string(), "./local.js".to_string()]
    );
}

#[test]
fn production_wrappers_do_not_contain_dev_bytes() {
    for file in reachable_files(USER_SPECS.iter().copied()) {
        let esm = esm_for_file(file).expect(file);
        assert!(
            !contains_dev_bytes(esm),
            "{file} leaked a development-build probe"
        );
        assert!(
            !esm.contains("NODE_ENV: \"development\""),
            "{file} is not a production wrap"
        );
    }
    let react = esm_for_file("react.js").unwrap();
    assert!(react.contains(REACT_VERSION));
    assert!(react.contains("useState"));
    let jsx = esm_for_file("jsx-runtime.js").unwrap();
    assert!(jsx.contains("exports.jsx"));
    assert!(jsx.contains("__dekaIslandJsx"));
    assert!(jsx.contains("deka-island"));
    let server = esm_for_file("react-dom-server.js").unwrap();
    assert!(server.contains("renderToString"));
    assert!(server.contains("renderToReadableStream"));
    let client = esm_for_file("react-dom-client.js").unwrap();
    assert!(client.contains("exports.createRoot"));
    assert!(client.contains("exports.hydrateRoot"));
    assert!(!client.contains("react-dom-client.development.js"));
}

#[test]
fn inline_rewrites_named_imports_and_keeps_prod_bytes() {
    let js = r#"
import { useState, createElement as h } from "@js/react";
import { jsx } from "@js/react/jsx-runtime";
import { renderToString } from "@js/react-dom/server";
export function probe() { return renderToString(jsx("div", { children: h("span") })); }
"#;
    let inlined = inline_into(js).expect("inline");
    assert!(!inlined.contains("from \"@js/react"));
    assert!(inlined.contains("__deka_js_builtins[\"react\"]"));
    assert!(inlined.contains("__deka_js_builtins[\"react/jsx-runtime\"]"));
    assert!(inlined.contains("__deka_js_builtins[\"react-dom/server\"]"));
    assert!(inlined.contains("const useState ="));
    assert!(inlined.contains("const h ="));
    assert!(inlined.contains("export function probe()"));
    assert!(!contains_dev_bytes(&inlined));
}

#[test]
fn rewrite_strips_auto_injected_react_import() {
    let src = "import { useState } from \"@js/react\"\n\nexport fn probe() string {\n  const [label, setLabel] = useState(\"ssr\")\n  return label\n}\n";
    let rewrite = rewrite_for_dsc_transpile(src);
    assert!(
        !rewrite.source.contains("@js/react"),
        "explicit @js/react import must be stripped: {}",
        rewrite.source
    );
    assert!(rewrite.source.contains("useState"));
    assert!(rewrite.stubs.is_empty());
}

#[test]
fn rewrite_stubs_react_dom_server_and_restores_the_specifier() {
    let src = "import { renderToString } from \"@js/react-dom/server\"\nexport fn html() string {\n  return renderToString(\"x\")\n}\n";
    let rewrite = rewrite_for_dsc_transpile(src);
    assert!(
        rewrite
            .source
            .contains("./__deka_js_builtin_react_dom_server.ds"),
        "{}",
        rewrite.source
    );
    assert!(!rewrite.source.contains("@js/react-dom/server"));
    assert_eq!(rewrite.stubs.len(), 1);
    assert!(rewrite.stubs[0].body.contains("export fn renderToString"));
    let restored = rewrite.restore_js(
        "import { renderToString } from \"./__deka_js_builtin_react_dom_server.js\";\n",
    );
    assert!(restored.contains("from \"@js/react-dom/server\""));
    assert!(!restored.contains("__deka_js_builtin_"));
}

#[test]
fn rewrite_stubs_react_dom_client() {
    let src = "import { hydrateRoot, createRoot } from \"@js/react-dom/client\"\nexport fn boot(node: ReactNode) ReactNode {\n  return node\n}\n";
    let rewrite = rewrite_for_dsc_transpile(src);
    assert!(
        rewrite
            .source
            .contains("./__deka_js_builtin_react_dom_client.ds"),
        "{}",
        rewrite.source
    );
    assert_eq!(rewrite.stubs.len(), 1);
    assert!(rewrite.stubs[0].body.contains("export fn hydrateRoot"));
    assert!(rewrite.stubs[0].body.contains("export fn createRoot"));
}

#[test]
fn inline_client_keeps_prod_bytes() {
    let js = r#"
import { hydrateRoot, createRoot } from "@js/react-dom/client";
import { jsx } from "@js/react/jsx-runtime";
export function boot(node) { return hydrateRoot(node, jsx("div", {})); }
"#;
    let inlined = inline_into(js).expect("inline");
    assert!(inlined.contains("__deka_js_builtins[\"react-dom/client\"]"));
    assert!(inlined.contains("__deka_js_builtins[\"scheduler\"]"));
    assert!(inlined.contains("const hydrateRoot ="));
    assert!(inlined.contains("const createRoot ="));
    assert!(!contains_dev_bytes(&inlined));
}
