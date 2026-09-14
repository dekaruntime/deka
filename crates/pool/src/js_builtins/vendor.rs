//! Pinned production CJS sources, content hashes, and named-export lists.
//!
//! Split out of `js_builtins.rs` (deka#391 file-size gate): the vendored
//! React production bytes and the lock-step SHA / export tables that wrap
//! them form one cohesive cluster.

pub const REACT_VERSION: &str = trim_version(include_str!("../../vendor/react-prod/VERSION"));

const fn trim_version(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        match bytes[end - 1] {
            b' ' | b'\n' | b'\r' | b'\t' => end -= 1,
            _ => break,
        }
    }
    let mut start = 0;
    while start < end {
        match bytes[start] {
            b' ' | b'\n' | b'\r' | b'\t' => start += 1,
            _ => break,
        }
    }
    let trimmed = raw.as_bytes().split_at(end).0.split_at(start).1;
    match core::str::from_utf8(trimmed) {
        Ok(text) => text,
        Err(_) => raw,
    }
}

#[cfg(test)]
pub(super) const HASHES: &str = include_str!("../../vendor/react-prod/HASHES");
#[cfg(test)]
pub(super) const MINIFIED_HASHES: &str = include_str!("../../vendor/react-prod/MINIFIED-HASHES");
pub(super) const REACT_CJS: &str = include_str!("../../vendor/react-prod/cjs/react.production.js");
pub(super) const JSX_RUNTIME_CJS: &str =
    include_str!("../../vendor/react-prod/cjs/react-jsx-runtime.production.js");
pub(super) const REACT_DOM_CJS: &str =
    include_str!("../../vendor/react-prod/cjs/react-dom.production.js");
pub(super) const REACT_DOM_CLIENT_CJS: &str =
    include_str!("../../vendor/react-prod/cjs/react-dom-client.production.js");
pub(super) const SCHEDULER_CJS: &str =
    include_str!("../../vendor/react-prod/cjs/scheduler.production.js");
pub(super) const SERVER_LEGACY_CJS: &str =
    include_str!("../../vendor/react-prod/cjs/react-dom-server-legacy.browser.production.js");
pub(super) const SERVER_BROWSER_CJS: &str =
    include_str!("../../vendor/react-prod/cjs/react-dom-server.edge.production.js");

pub(super) const REACT_SHA: &str =
    "f1e2323f141be9d9379c612eeabd7f282f052e60116547395780b032a6f0770d";
pub(super) const JSX_RUNTIME_SHA: &str =
    "1e46f15002696985e80c61d47aaa30dabb03c954270a690f5f7dfbbacfa9002b";
pub(super) const REACT_DOM_SHA: &str =
    "f518694f8588dacc9acf35452bbeb18f98174bc2e3eb1bf1181b7fbb4a0f0295";
pub(super) const REACT_DOM_CLIENT_SHA: &str =
    "b42d9ab70da856b96e240e3d010b08f4284113cf568b1b2a2aa1f251977eeb37";
pub(super) const SCHEDULER_SHA: &str =
    "679adff761d31e9426604f80c0e99be44a3e6f4c6834b0218ecf0312a67c171f";
pub(super) const SERVER_LEGACY_SHA: &str =
    "cf3928705cd3051cb8fb88da14811aee4ac4320e4eb4caa5193996019d7cf008";
pub(super) const SERVER_BROWSER_SHA: &str =
    "8b316fa071e1fb0e4eb30ae1783c3a16a5d67ca0736192bedda6b5c03310cb7d";

pub(super) const REACT_EXPORTS: &[&str] = &[
    "Children",
    "Component",
    "Fragment",
    "Profiler",
    "PureComponent",
    "StrictMode",
    "Suspense",
    "__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE",
    "__COMPILER_RUNTIME",
    "cache",
    "cloneElement",
    "createContext",
    "createElement",
    "createRef",
    "forwardRef",
    "isValidElement",
    "lazy",
    "memo",
    "startTransition",
    "unstable_useCacheRefresh",
    "use",
    "useActionState",
    "useCallback",
    "useContext",
    "useDebugValue",
    "useDeferredValue",
    "useEffect",
    "useId",
    "useImperativeHandle",
    "useInsertionEffect",
    "useLayoutEffect",
    "useMemo",
    "useOptimistic",
    "useReducer",
    "useRef",
    "useState",
    "useSyncExternalStore",
    "useTransition",
    "version",
];
pub(super) const JSX_RUNTIME_EXPORTS: &[&str] = &["Fragment"];
pub(super) const REACT_DOM_CLIENT_EXPORTS: &[&str] = &["createRoot", "hydrateRoot", "version"];
pub(super) const SCHEDULER_EXPORTS: &[&str] = &[
    "unstable_IdlePriority",
    "unstable_ImmediatePriority",
    "unstable_LowPriority",
    "unstable_NormalPriority",
    "unstable_Profiling",
    "unstable_UserBlockingPriority",
    "unstable_cancelCallback",
    "unstable_forceFrameRate",
    "unstable_getCurrentPriorityLevel",
    "unstable_next",
    "unstable_now",
    "unstable_requestPaint",
    "unstable_runWithPriority",
    "unstable_scheduleCallback",
    "unstable_shouldYield",
    "unstable_wrapCallback",
];
pub(super) const REACT_DOM_EXPORTS: &[&str] = &[
    "__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE",
    "createPortal",
    "flushSync",
    "preconnect",
    "prefetchDNS",
    "preinit",
    "preinitModule",
    "preload",
    "preloadModule",
    "requestFormReset",
    "unstable_batchedUpdates",
    "useFormState",
    "useFormStatus",
    "version",
];
pub(super) const SERVER_LEGACY_EXPORTS: &[&str] =
    &["renderToStaticMarkup", "renderToString", "version"];
pub(super) const SERVER_BROWSER_EXPORTS: &[&str] =
    &["prerender", "renderToReadableStream", "version"];

pub(super) fn registry_key(file: &str) -> &'static str {
    match file {
        "react.js" => "react",
        "jsx-runtime.js" => "react/jsx-runtime",
        "react-dom.js" => "react-dom",
        "react-dom-client.js" => "react-dom/client",
        "scheduler.js" => "scheduler",
        "react-dom-server-legacy.js" => "react-dom-server-legacy",
        "react-dom-server-browser.js" => "react-dom-server-browser",
        "react-dom-server.js" => "react-dom/server",
        _ => "unknown",
    }
}

pub(super) fn cjs_for_file(file: &str) -> Option<(&'static str, &'static str)> {
    match file {
        "react.js" => Some((vendor_cjs("react.production.js", REACT_CJS), REACT_SHA)),
        "jsx-runtime.js" => Some((
            vendor_cjs("react-jsx-runtime.production.js", JSX_RUNTIME_CJS),
            JSX_RUNTIME_SHA,
        )),
        "react-dom.js" => Some((
            vendor_cjs("react-dom.production.js", REACT_DOM_CJS),
            REACT_DOM_SHA,
        )),
        "react-dom-client.js" => Some((
            vendor_cjs("react-dom-client.production.js", REACT_DOM_CLIENT_CJS),
            REACT_DOM_CLIENT_SHA,
        )),
        "scheduler.js" => Some((
            vendor_cjs("scheduler.production.js", SCHEDULER_CJS),
            SCHEDULER_SHA,
        )),
        "react-dom-server-legacy.js" => Some((
            vendor_cjs(
                "react-dom-server-legacy.browser.production.js",
                SERVER_LEGACY_CJS,
            ),
            SERVER_LEGACY_SHA,
        )),
        "react-dom-server-browser.js" => Some((
            vendor_cjs("react-dom-server.edge.production.js", SERVER_BROWSER_CJS),
            SERVER_BROWSER_SHA,
        )),
        _ => None,
    }
}

pub(super) fn vendor_cjs(name: &'static str, source: &'static str) -> &'static str {
    crate::js_minify::cached(name, source)
}
