/**
 * Shared utility-CSS scanner/generator for deka.
 *
 * Consumes a JSON registry and produces CSS only for the classes actually
 * present in a chunk of HTML. Used by both the browser tour preview and the
 * server-side runtime (via deno_core).
 */

export interface UtilityCssRegistry {
  preflight: string
  scales: Record<string, Record<string, string>>
  utilities: Record<string, UtilityRule | UtilityRule[]>
  variants: Record<string, Variant>
}

interface UtilityRule {
  props?: string[]
  /** Literal values keyed by token, checked before scales/arbitrary values. */
  values?: Record<string, string>
  /** Name of a scale in `scales` to resolve the token against. */
  scale?: string
  /**
   * When the scale value contains a `/`, treat it as a shorthand for multiple
   * properties, e.g. "0.75rem/1rem" -> font-size:0.75rem;line-height:1rem;
   */
  split?: '/' | '_'
  /** Emit a static declaration regardless of the token. */
  static?: string
  /** Template string with `{value}` placeholder for arbitrary/computed values. */
  template?: string
  /** Template used specifically for arbitrary `[...]` values. */
  arbitraryTemplate?: string
  /** Allow arbitrary values such as `w-[100px]`. */
  arbitrary?: boolean
  /**
   * Optional color fallback: if the token matches a value in the `colors`
   * scale, emit these properties instead. Used for ambiguous prefixes like
   * `border-*` (width/style vs color).
   */
  color?: {
    props: string[]
    scale: string
  }
}

interface Variant {
  selector?: string
  media?: string
}

const MARKER = '__deka_utility_css'

export interface InjectOptions {
  enabled?: boolean
  includePreflight?: boolean
}

export function injectUtilityCss(
  html: string,
  registryJson: string,
  options: InjectOptions = {}
): string {
  const { enabled = true, includePreflight = true } = options
  if (!enabled || html.includes(MARKER)) {
    return html
  }

  const registry: UtilityCssRegistry = JSON.parse(registryJson)
  const classes = collectClasses(html)
  if (classes.size === 0) {
    return html
  }

  const css = generateCss(classes, registry)
  if (!css) {
    return html
  }

  const preflight = includePreflight ? registry.preflight : ''
  const style = `<style id="${MARKER}">${preflight}${css}</style>`

  const headClose = html.toLowerCase().lastIndexOf('</head>')
  if (headClose >= 0) {
    return html.slice(0, headClose) + style + html.slice(headClose)
  }
  return style + html
}

export function generateUtilityCss(
  html: string,
  registryJson: string,
  options: InjectOptions = {}
): string {
  return injectUtilityCss(html, registryJson, options)
}

export function collectClasses(html: string): Set<string> {
  const out = new Set<string>()
  const regex = /class=(["'])(.*?)\1/g
  let match: RegExpExecArray | null
  while ((match = regex.exec(html)) !== null) {
    for (const token of match[2].split(/\s+/)) {
      if (token) out.add(token)
    }
  }
  return out
}

function generateCss(classes: Set<string>, registry: UtilityCssRegistry): string {
  const rules: string[] = []
  for (const cls of classes) {
    const rule = classToRule(cls, registry)
    if (rule) rules.push(rule)
  }
  return rules.join('')
}

function classToRule(cls: string, registry: UtilityCssRegistry): string | null {
  const parts = cls.split(':')
  if (parts.length === 0) return null
  const base = parts.pop()!

  let selector = `.${escapeSelector(cls)}`
  let media: string | null = null

  for (const variant of parts) {
    const v = registry.variants[variant]
    if (!v) return null
    if (v.selector) {
      selector = applyVariantSelector(selector, v.selector)
    }
    if (v.media) {
      media = v.media
    }
  }

  const decl = baseToDecl(base, registry)
  if (!decl) return null

  const rule = `${selector}{${decl}}`
  return media ? `@media (${media}){${rule}}` : rule
}

function applyVariantSelector(selector: string, variant: string): string {
  if (variant.includes('&')) {
    return variant.replace(/&/g, selector)
  }
  return `${variant} ${selector}`
}

function baseToDecl(base: string, registry: UtilityCssRegistry): string | null {
  // Build candidate prefixes from longest to shortest so multi-word utilities
  // like `grid-cols-*` win over bare `grid`.
  const candidates: string[] = []
  const hyphenPositions: number[] = []
  let idx = base.indexOf('-')
  while (idx > 0) {
    hyphenPositions.push(idx)
    idx = base.indexOf('-', idx + 1)
  }
  // Longest prefix first: base, then each prefix from the last hyphen back.
  candidates.push(base)
  for (let i = hyphenPositions.length - 1; i >= 0; i--) {
    candidates.push(base.slice(0, hyphenPositions[i]))
  }

  for (const prefix of candidates) {
    const rule = registry.utilities[prefix]
    if (!rule) continue

    const token = prefix === base ? '' : base.slice(prefix.length + 1)
    const decl = ruleToDecl(rule, token, base, registry)
    if (decl) return decl
  }

  return null
}

function ruleToDecl(
  rule: UtilityRule | UtilityRule[],
  token: string,
  base: string,
  registry: UtilityCssRegistry
): string | null {
  if (Array.isArray(rule)) {
    for (const r of rule) {
      const decl = singleRuleToDecl(r, token, base, registry)
      if (decl) return decl
    }
    return null
  }
  return singleRuleToDecl(rule, token, base, registry)
}

function singleRuleToDecl(
  rule: UtilityRule,
  token: string,
  base: string,
  registry: UtilityCssRegistry
): string | null {
  // Ambiguous prefixes like `border-*` may refer to a color. Check that first.
  if (rule.color && token) {
    const colorScale = registry.scales[rule.color.scale]
    if (colorScale && token in colorScale) {
      const value = colorScale[token]
      return rule.color.props.map((p) => `${p}:${value}`).join(';') + ';'
    }
  }

  if (rule.static) {
    if (rule.props) {
      const pieces = rule.static!.includes(' ') ? rule.static!.split(' ') : []
      if (pieces.length === rule.props!.length) {
        return rule.props!.map((p, i) => `${p}:${pieces[i]}`).join(';') + ';'
      }
      return rule.props!.map((p) => `${p}:${rule.static}`).join(';') + ';'
    }
    return rule.static
  }

  if (rule.template) {
    const isArbitrary = token.startsWith('[') && token.endsWith(']')
    if (isArbitrary && rule.arbitraryTemplate) {
      const inner = token.slice(1, -1).replace(/_/g, ' ')
      if (inner) return rule.arbitraryTemplate.replace(/\{value\}/g, inner)
    }

    let value = resolveValue(rule, token, base, registry)
    if (value === null && token) {
      // Template utilities such as grid-cols-3 accept any non-empty token.
      value = token
    }
    if (value === null) return null
    return rule.template.replace(/\{value\}/g, value)
  }

  const value = resolveValue(rule, token, base, registry)
  if (value === null) return null

  const props = rule.props
  if (!props || props.length === 0) return null

  if (rule.split && value.includes(rule.split)) {
    const pieces = value.split(rule.split)
    if (pieces.length === props.length) {
      return props.map((p, i) => `${p}:${pieces[i]}`).join(';') + ';'
    }
  }

  if (props.length === 1) {
    return `${props[0]}:${value};`
  }

  return props.map((p) => `${p}:${value}`).join(';') + ';'
}

function resolveValue(
  rule: UtilityRule,
  token: string,
  base: string,
  registry: UtilityCssRegistry
): string | null {
  // 1. Literal values map.
  if (rule.values && token in rule.values) {
    return rule.values[token]
  }

  // 2. Named scale lookup.
  if (rule.scale && token) {
    const scale = registry.scales[rule.scale]
    if (scale && token in scale) {
      return scale[token]
    }
  }

  // 3. Arbitrary values, e.g. w-[100px], grid-cols-[1fr_2fr]
  if (rule.arbitrary !== false && token.startsWith('[') && token.endsWith(']')) {
    const inner = token.slice(1, -1).replace(/_/g, ' ')
    if (inner) return inner
  }

  // 4. Standalone utilities with no token (e.g. "flex", "hidden").
  if (!token && rule.static === undefined) {
    // Some rules are just a marker that needs a token; don't guess.
    return null
  }

  return null
}

function escapeSelector(cls: string): string {
  return cls
    .split('')
    .map((ch) => {
      if (/[a-zA-Z0-9_-]/.test(ch)) return ch
      return `\\${ch}`
    })
    .join('')
}

// Expose on globalThis so the bundled IIFE can be loaded directly into a JS
// engine (deno_core, browser script tag) without an ESM loader.
if (typeof globalThis !== 'undefined') {
  ;(globalThis as any).dekaUtilityCss = {
    injectUtilityCss,
    generateUtilityCss,
    collectClasses,
  }
}
