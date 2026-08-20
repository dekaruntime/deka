(() => {
  var __defProp = Object.defineProperty;
  var __getOwnPropNames = Object.getOwnPropertyNames;
  var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
  var __hasOwnProp = Object.prototype.hasOwnProperty;
  function __accessProp(key) {
    return this[key];
  }
  var __toCommonJS = (from) => {
    var entry = (__moduleCache ??= new WeakMap).get(from), desc;
    if (entry)
      return entry;
    entry = __defProp({}, "__esModule", { value: true });
    if (from && typeof from === "object" || typeof from === "function") {
      for (var key of __getOwnPropNames(from))
        if (!__hasOwnProp.call(entry, key))
          __defProp(entry, key, {
            get: __accessProp.bind(from, key),
            enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable
          });
    }
    __moduleCache.set(from, entry);
    return entry;
  };
  var __moduleCache;
  var __returnValue = (v) => v;
  function __exportSetter(name, newValue) {
    this[name] = __returnValue.bind(null, newValue);
  }
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, {
        get: all[name],
        enumerable: true,
        configurable: true,
        set: __exportSetter.bind(all, name)
      });
  };

  // index.ts
  var exports_utility_css = {};
  __export(exports_utility_css, {
    collectClasses: () => collectClasses,
    generateUtilityCss: () => generateUtilityCss,
    injectUtilityCss: () => injectUtilityCss
  });
  var MARKER = "__deka_utility_css";
  function injectUtilityCss(html, registryJson, options = {}) {
    const { enabled = true, includePreflight = true } = options;
    if (!enabled || html.includes(MARKER)) {
      return html;
    }
    const registry = JSON.parse(registryJson);
    const classes = collectClasses(html);
    if (classes.size === 0) {
      return html;
    }
    const css = generateCss(classes, registry);
    if (!css) {
      return html;
    }
    const preflight = includePreflight ? registry.preflight : "";
    const style = `<style id="${MARKER}">${preflight}${css}</style>`;
    const headClose = html.toLowerCase().lastIndexOf("</head>");
    if (headClose >= 0) {
      return html.slice(0, headClose) + style + html.slice(headClose);
    }
    return style + html;
  }
  function generateUtilityCss(html, registryJson, options = {}) {
    return injectUtilityCss(html, registryJson, options);
  }
  function collectClasses(html) {
    const out = new Set;
    const regex = /class=(["'])(.*?)\1/g;
    let match;
    while ((match = regex.exec(html)) !== null) {
      for (const token of match[2].split(/\s+/)) {
        if (token)
          out.add(token);
      }
    }
    return out;
  }
  function generateCss(classes, registry) {
    const rules = [];
    for (const cls of classes) {
      const rule = classToRule(cls, registry);
      if (rule)
        rules.push(rule);
    }
    return rules.join("");
  }
  function classToRule(cls, registry) {
    const parts = cls.split(":");
    if (parts.length === 0)
      return null;
    const base = parts.pop();
    let selector = `.${escapeSelector(cls)}`;
    let media = null;
    for (const variant of parts) {
      const v = registry.variants[variant];
      if (!v)
        return null;
      if (v.selector) {
        selector = applyVariantSelector(selector, v.selector);
      }
      if (v.media) {
        media = v.media;
      }
    }
    const decl = baseToDecl(base, registry);
    if (!decl)
      return null;
    const rule = `${selector}{${decl}}`;
    return media ? `@media (${media}){${rule}}` : rule;
  }
  function applyVariantSelector(selector, variant) {
    if (variant.includes("&")) {
      return variant.replace(/&/g, selector);
    }
    return `${variant} ${selector}`;
  }
  function baseToDecl(base, registry) {
    const candidates = [];
    const hyphenPositions = [];
    let idx = base.indexOf("-");
    while (idx > 0) {
      hyphenPositions.push(idx);
      idx = base.indexOf("-", idx + 1);
    }
    candidates.push(base);
    for (let i = hyphenPositions.length - 1;i >= 0; i--) {
      candidates.push(base.slice(0, hyphenPositions[i]));
    }
    for (const prefix of candidates) {
      const rule = registry.utilities[prefix];
      if (!rule)
        continue;
      const token = prefix === base ? "" : base.slice(prefix.length + 1);
      const decl = ruleToDecl(rule, token, base, registry);
      if (decl)
        return decl;
    }
    return null;
  }
  function ruleToDecl(rule, token, base, registry) {
    if (Array.isArray(rule)) {
      for (const r of rule) {
        const decl = singleRuleToDecl(r, token, base, registry);
        if (decl)
          return decl;
      }
      return null;
    }
    return singleRuleToDecl(rule, token, base, registry);
  }
  function singleRuleToDecl(rule, token, base, registry) {
    if (rule.color && token) {
      const colorScale = registry.scales[rule.color.scale];
      if (colorScale && token in colorScale) {
        const value2 = colorScale[token];
        return rule.color.props.map((p) => `${p}:${value2}`).join(";") + ";";
      }
    }
    if (rule.static) {
      if (rule.props) {
        const pieces = rule.static.includes(" ") ? rule.static.split(" ") : [];
        if (pieces.length === rule.props.length) {
          return rule.props.map((p, i) => `${p}:${pieces[i]}`).join(";") + ";";
        }
        return rule.props.map((p) => `${p}:${rule.static}`).join(";") + ";";
      }
      return rule.static;
    }
    if (rule.template) {
      const isArbitrary = token.startsWith("[") && token.endsWith("]");
      if (isArbitrary && rule.arbitraryTemplate) {
        const inner = token.slice(1, -1).replace(/_/g, " ");
        if (inner)
          return rule.arbitraryTemplate.replace(/\{value\}/g, inner);
      }
      let value2 = resolveValue(rule, token, base, registry);
      if (value2 === null && token) {
        value2 = token;
      }
      if (value2 === null)
        return null;
      return rule.template.replace(/\{value\}/g, value2);
    }
    const value = resolveValue(rule, token, base, registry);
    if (value === null)
      return null;
    const props = rule.props;
    if (!props || props.length === 0)
      return null;
    if (rule.split && value.includes(rule.split)) {
      const pieces = value.split(rule.split);
      if (pieces.length === props.length) {
        return props.map((p, i) => `${p}:${pieces[i]}`).join(";") + ";";
      }
    }
    if (props.length === 1) {
      return `${props[0]}:${value};`;
    }
    return props.map((p) => `${p}:${value}`).join(";") + ";";
  }
  function resolveValue(rule, token, base, registry) {
    if (rule.values && token in rule.values) {
      return rule.values[token];
    }
    if (rule.scale && token) {
      const scale = registry.scales[rule.scale];
      if (scale && token in scale) {
        return scale[token];
      }
    }
    if (rule.arbitrary !== false && token.startsWith("[") && token.endsWith("]")) {
      const inner = token.slice(1, -1).replace(/_/g, " ");
      if (inner)
        return inner;
    }
    if (!token && rule.static === undefined) {
      return null;
    }
    return null;
  }
  function escapeSelector(cls) {
    return cls.split("").map((ch) => {
      if (/[a-zA-Z0-9_-]/.test(ch))
        return ch;
      return `\\${ch}`;
    }).join("");
  }
  if (typeof globalThis !== "undefined") {
    globalThis.dekaUtilityCss = {
      injectUtilityCss,
      generateUtilityCss,
      collectClasses
    };
  }
})();

;globalThis.__dekaGenerateUtilityCss = (html, registryJson, options) => { return globalThis.dekaUtilityCss.injectUtilityCss(html, registryJson, options);};
