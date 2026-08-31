/**
 * Builds the CommonJS-shaped loader that the browser harness evaluates.
 *
 * Pure string construction, deliberately free of playwright and web-ide-kit
 * imports so it can be unit tested without a browser or the compiler.
 */

function normalizePath(filePath: string): string {
  return filePath.replace(/\\/g, '/').replace(/^\.\//, '')
}

export function projectLoaderJs(
  entryPath: string,
  modules: Record<string, { code: string }>
): string {
  const normalizedEntry = normalizePath(entryPath)
  const moduleEntries = Object.entries(modules).map(([modulePath, module]) => {
    const safePath = JSON.stringify(modulePath)
    // The module body is spliced in as *raw JavaScript source* — it becomes the
    // body of a function in an object literal. It is NOT inside a template
    // literal, so it must not be escaped for one. Escaping here produced
    // `SyntaxError: Invalid or unexpected token` for any module containing a
    // backtick or `${` (i.e. every template literal a user writes, and the
    // struct prelude), and silently corrupted `\n` into a literal backslash-n
    // everywhere else.
    return `  ${safePath}: function(exports, __dekaRequire, module) {\n${module.code}\n}`
  })
  return `
const __dekaModules = {\n${moduleEntries.join(',\n')}\n};
const __dekaCache = new Map();
function __dekaResolve(spec, currentPath) {
  if (!spec.startsWith('./') && !spec.startsWith('../')) return spec;
  const base = currentPath.includes('/') ? currentPath.slice(0, currentPath.lastIndexOf('/') + 1) : '';
  const parts = (base + spec).split('/').filter(Boolean);
  const resolved = [];
  for (const part of parts) {
    if (part === '..') resolved.pop();
    else if (part !== '.') resolved.push(part);
  }
  return resolved.join('/');
}
function __dekaRequire(spec, currentPath) {
  const normalized = __dekaResolve(spec, currentPath || ${JSON.stringify(normalizedEntry)});
  if (__dekaCache.has(normalized)) return __dekaCache.get(normalized);
  const factory = __dekaModules[normalized];
  if (!factory) throw new Error('Module not found: ' + spec + ' (resolved to ' + normalized + ')');
  const module = { exports: {} };
  factory(module.exports, (s) => __dekaRequire(s, normalized), module);
  __dekaCache.set(normalized, module.exports);
  return module.exports;
}
__dekaRequire(${JSON.stringify(normalizedEntry)});
`
}
