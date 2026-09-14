//! Island JSX wrapping and live-signal SSR prerender for `@js/react/jsx-runtime`.
//!
//! Split out of `js_builtins.rs` (deka#391 file-size gate): the `client:*`
//! host wrap and live-interpolation evaluation form one cohesive cluster
//! that must stay in lockstep with `hydrateRoot`'s snapshot.

/// Wrap `client:*` function components in a `<deka-island>` host so
/// `hydrateRoot` has a container. Host tags keep the prop; the island
/// component itself is rendered as the custom element's child.
///
/// Live interpolations (`{ __live, read }`, the `ui/reactive` `live()`
/// node) are evaluated here so SSR writes the current signal value into
/// the HTML. `hydrateRoot` uses the same factory, so the client snapshot
/// matches. Throws stay loud (deka#746 F3).
pub(super) const ISLAND_JSX_WRAP: &str = r#"
const __jsx = __m.jsx;
const __jsxs = __m.jsxs;
function __dekaResolveChild(child) {
  if (child == null || typeof child === "boolean") return child;
  if (typeof child === "object") {
    if (child.__live === true && typeof child.read === "function") {
      return __dekaResolveChild(child.read());
    }
    if (Array.isArray(child)) {
      let changed = false;
      const out = child.map(function (item) {
        const next = __dekaResolveChild(item);
        if (next !== item) changed = true;
        return next;
      });
      return changed ? out : child;
    }
  }
  return child;
}
function __dekaResolveConfig(config) {
  if (config == null || typeof config !== "object") return config;
  if (!Object.prototype.hasOwnProperty.call(config, "children")) return config;
  const resolved = __dekaResolveChild(config.children);
  if (resolved === config.children) return config;
  const next = {};
  for (const k in config) {
    if (Object.prototype.hasOwnProperty.call(config, k)) next[k] = config[k];
  }
  next.children = resolved;
  return next;
}
function __dekaIslandJsx(factory, type, config, key) {
  config = __dekaResolveConfig(config);
  if (config != null && typeof type === "function") {
    const load = config["client:load"];
    const idle = config["client:idle"];
    const visible = config["client:visible"];
    if (load || idle || visible) {
      const directive = load ? "load" : idle ? "idle" : "visible";
      const inner = {};
      const serializable = {};
      for (const k in config) {
        if (!Object.prototype.hasOwnProperty.call(config, k)) continue;
        if (k === "client:load" || k === "client:idle" || k === "client:visible") continue;
        inner[k] = config[k];
        if (k !== "children" && k !== "key" && k !== "ref") {
          const v = config[k];
          const t = typeof v;
          if (v == null || (t !== "function" && t !== "symbol")) serializable[k] = v;
        }
      }
      const name = type.displayName || type.name || "Island";
      return factory("deka-island", {
        "data-deka-island": name,
        "data-deka-directive": directive,
        "data-deka-props": JSON.stringify(serializable),
        style: { display: "contents" },
        children: factory(type, inner)
      }, key);
    }
  }
  return factory(type, config, key);
}
export const jsx = (type, config, key) => __dekaIslandJsx(__jsx, type, config, key);
export const jsxs = (type, config, key) => __dekaIslandJsx(__jsxs, type, config, key);
"#;
