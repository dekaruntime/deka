// ui/jsx — ComponentNode factories. Must stay tiny and must never import
// ui/server. Function tags are not invoked here; the renderer calls them.

export const Fragment = Symbol.for("deka.ui.Fragment");

export function isComponentNode(value) {
  return value != null && typeof value === "object" && value.__componentNode === true;
}

export function normalizeJsxChildren(children) {
  if (children == null) return [];
  if (Array.isArray(children)) {
    const out = [];
    for (const child of children) {
      if (child == null || child === false || child === true) continue;
      if (Array.isArray(child)) {
        for (const inner of normalizeJsxChildren(child)) out.push(inner);
      } else {
        out.push(child);
      }
    }
    return out;
  }
  if (children === false || children === true) return [];
  return [children];
}

// The `__componentNode` marker is an enumerable literal field, not a hidden
// defineProperty: hidden properties force every node into dictionary mode,
// which measured ~12us/render slower on a 49-node grid (deka#580). Nothing
// serializes or spreads whole nodes, so enumerability is safe (jsonSafe maps
// nodes to undefined by reading this flag).
function createComponentNode(tag, props, children) {
  if (children === undefined) {
    // Legacy form: children carried inside props (userland jsx() calls such
    // as ui/jsx consumers, or spread attributes). Strip them so props and
    // children stay separate for the renderers.
    const input = props ?? {};
    if (Object.hasOwn(input, "children")) {
      const { children: nested, ...rest } = input;
      children = normalizeJsxChildren(nested);
      props = rest;
    } else {
      children = [];
      props = input;
    }
  } else if (Array.isArray(children)) {
    // Compiler-emitted arrays are flat and hole-free unless a conditional or
    // spread child produced null/boolean/nested values; only then copy.
    let needsNormalize = false;
    for (const child of children) {
      if (child == null || child === true || child === false || Array.isArray(child)) {
        needsNormalize = true;
        break;
      }
    }
    if (needsNormalize) children = normalizeJsxChildren(children);
  } else if (children === true || children === false || children == null) {
    children = [];
  } else {
    children = [children];
  }
  return { tag, props, children, __componentNode: true };
}

export function jsx(tag, props, children) {
  return createComponentNode(tag, props, children);
}

export const jsxs = jsx;
