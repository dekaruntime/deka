// Minimal PHP runtime module - no Node.js compatibility
// This provides only the essentials for PHP execution

const { op_php_read_file_sync, op_php_cwd, op_php_file_exists, op_php_path_resolve, op_php_set_privileged } = Deno.core.ops;

// Basic console implementation
const console = {
  log: (...args) => {
    const message = args.map(a => String(a)).join(' ');
    Deno.core.print(message + '\n', false);
  },
  error: (...args) => {
    const message = args.map(a => String(a)).join(' ');
    Deno.core.print(message + '\n', true);
  },
  warn: (...args) => {
    const message = '[WARN] ' + args.map(a => String(a)).join(' ');
    Deno.core.print(message + '\n', true);
  },
  info: (...args) => {
    const message = '[INFO] ' + args.map(a => String(a)).join(' ');
    Deno.core.print(message + '\n', false);
  },
  debug: (...args) => {
    const message = '[DEBUG] ' + args.map(a => String(a)).join(' ');
    Deno.core.print(message + '\n', false);
  },
};

globalThis.console = console;

// Basic TextEncoder/TextDecoder
if (!globalThis.TextEncoder) {
  globalThis.TextEncoder = class TextEncoder {
    encode(input) {
      const str = String(input);
      const utf8 = [];
      for (let i = 0; i < str.length; i++) {
        let charCode = str.charCodeAt(i);
        if (charCode < 0x80) {
          utf8.push(charCode);
        } else if (charCode < 0x800) {
          utf8.push(0xc0 | (charCode >> 6), 0x80 | (charCode & 0x3f));
        } else if (charCode < 0xd800 || charCode >= 0xe000) {
          utf8.push(0xe0 | (charCode >> 12), 0x80 | ((charCode >> 6) & 0x3f), 0x80 | (charCode & 0x3f));
        } else {
          i++;
          charCode = 0x10000 + (((charCode & 0x3ff) << 10) | (str.charCodeAt(i) & 0x3ff));
          utf8.push(
            0xf0 | (charCode >> 18),
            0x80 | ((charCode >> 12) & 0x3f),
            0x80 | ((charCode >> 6) & 0x3f),
            0x80 | (charCode & 0x3f)
          );
        }
      }
      return new Uint8Array(utf8);
    }
  };
}

if (!globalThis.TextDecoder) {
  globalThis.TextDecoder = class TextDecoder {
    decode(bytes) {
      if (!bytes) return '';
      const arr = new Uint8Array(bytes);
      let str = '';
      let i = 0;
      while (i < arr.length) {
        let byte = arr[i++];
        if (byte < 0x80) {
          str += String.fromCharCode(byte);
        } else if (byte < 0xe0) {
          str += String.fromCharCode(((byte & 0x1f) << 6) | (arr[i++] & 0x3f));
        } else if (byte < 0xf0) {
          str += String.fromCharCode(
            ((byte & 0x0f) << 12) | ((arr[i++] & 0x3f) << 6) | (arr[i++] & 0x3f)
          );
        } else {
          const code =
            ((byte & 0x07) << 18) |
            ((arr[i++] & 0x3f) << 12) |
            ((arr[i++] & 0x3f) << 6) |
            (arr[i++] & 0x3f);
          const high = ((code - 0x10000) >> 10) | 0xd800;
          const low = ((code - 0x10000) & 0x3ff) | 0xdc00;
          str += String.fromCharCode(high, low);
        }
      }
      return str;
    }
  };
}

// Minimal fs implementation for PHP wasm loading
if (!globalThis.fs) {
  globalThis.fs = {};
}
if (!globalThis.fs.readFileSync) {
  globalThis.fs.readFileSync = (path, encoding) => {
    const bytes = op_php_read_file_sync(path);
    if (encoding === 'utf8' || encoding === 'utf-8') {
      return new TextDecoder().decode(bytes);
    }
    return bytes;
  };
}
if (!globalThis.fs.existsSync) {
  globalThis.fs.existsSync = (path) => {
    return op_php_file_exists(path);
  };
}

// __dekaFs - tenant-root-confined wrapper around globalThis.fs.
//
// Motivation: globalThis.fs methods call op_php_* ops which are
// security-policy-gated, but a tenant can call op_php_set_privileged(1)
// via Deno.core.ops to bypass policy for the current thread. Wrapping
// at the JS layer adds a path-prefix check that cannot be bypassed via
// the privileged flag: if the resolved path does not start with the
// tenant root, the call is rejected before reaching any op.
//
// The tenant root is baked into each tenant bundle as:
//   globalThis.__dekaFsTenantRoot = "/absolute/project/root";
// (injected by js_pipeline.rs / build_phpx_handler_bundle).
// This module reads it lazily (at call time) so the wrapper is safe to
// install during extension init, before the bundle has run.
//
// Three-layer guard (mirrors esm_loader.rs @/ path-traversal fix):
//   1. Reject any path segment equal to ".." or "." before IO.
//   2. Resolve the path against the cwd (normalise without IO).
//   3. Assert the resolved path starts with the canonicalised tenant root.
//
// Fail-closed: if tenantRoot is not set, all calls return null/false/void.
// This keeps the polyfills inert in contexts where no root is configured
// (e.g. bare deno/test environments) rather than silently allowing full-fs
// access.
(function installDekaFsWrapper() {
  function dekaFsGetRoot() {
    const r = globalThis.__dekaFsTenantRoot;
    if (typeof r !== 'string' || !r) return null;
    // Ensure trailing separator so starts_with is unambiguous.
    return r.endsWith('/') ? r : r + '/';
  }

  function dekaFsNormPath(p) {
    // Resolve relative paths against cwd using the op.
    let resolved;
    try {
      resolved = typeof p === 'string' && p.startsWith('/')
        ? p
        : (op_php_cwd() + '/' + p);
    } catch (_) {
      return null;
    }
    // Collapse ./ and // without resolving symlinks.
    const parts = resolved.split('/');
    const out = [];
    for (const seg of parts) {
      if (seg === '' || seg === '.') continue;
      if (seg === '..') { out.pop(); continue; }
      out.push(seg);
    }
    return '/' + out.join('/');
  }

  function dekaFsAllow(p) {
    const root = dekaFsGetRoot();
    // Fail-closed: no root configured => deny.
    if (!root) return false;
    const rawStr = String(p || '');
    // Reject literal traversal segments before any resolution (layer 1).
    const segs = rawStr.replace(/\\/g, '/').split('/');
    for (const seg of segs) {
      if (seg === '..' || (seg === '.' && segs.length > 1)) return false;
    }
    // Normalise (layer 2).
    const norm = dekaFsNormPath(rawStr);
    if (!norm) return false;
    // Prefix assertion (layer 3): path must start with tenant root.
    // Add trailing slash to norm for unambiguous prefix check.
    const normSlash = norm.endsWith('/') ? norm : norm + '/';
    return normSlash.startsWith(root) || norm === root.slice(0, -1);
  }

  // Install __dekaFs only if not already installed (idempotent).
  if (typeof globalThis.__dekaFs === 'undefined') {
    globalThis.__dekaFs = {
      statSync: (p) => {
        if (!dekaFsAllow(p)) return null;
        try {
          const raw = globalThis.fs;
          if (raw && typeof raw.statSync === 'function') return raw.statSync(String(p));
        } catch (_) {}
        return null;
      },
      readFileSync: (p, encoding) => {
        if (!dekaFsAllow(p)) return null;
        try {
          const raw = globalThis.fs;
          if (raw && typeof raw.readFileSync === 'function') return raw.readFileSync(String(p), encoding);
        } catch (_) {}
        return null;
      },
      writeFileSync: (p, data, encoding) => {
        if (!dekaFsAllow(p)) return;
        try {
          const raw = globalThis.fs;
          if (raw && typeof raw.writeFileSync === 'function') return raw.writeFileSync(String(p), data, encoding);
        } catch (_) {}
      },
      mkdirSync: (p, options) => {
        if (!dekaFsAllow(p)) return;
        try {
          const raw = globalThis.fs;
          if (raw && typeof raw.mkdirSync === 'function') return raw.mkdirSync(String(p), options);
        } catch (_) {}
      },
      appendFileSync: (p, data) => {
        if (!dekaFsAllow(p)) return;
        try {
          const raw = globalThis.fs;
          if (raw && typeof raw.appendFileSync === 'function') return raw.appendFileSync(String(p), data);
        } catch (_) {}
      },
      readdirSync: (p, options) => {
        if (!dekaFsAllow(p)) return null;
        try {
          const raw = globalThis.fs;
          if (raw && typeof raw.readdirSync === 'function') return raw.readdirSync(String(p), options);
        } catch (_) {}
        return null;
      },
      existsSync: (p) => {
        if (!dekaFsAllow(p)) return false;
        try {
          const raw = globalThis.fs;
          if (raw && typeof raw.existsSync === 'function') return raw.existsSync(String(p));
        } catch (_) {}
        return false;
      },
    };
  }
}());

// Minimal process implementation
if (!globalThis.process) {
  globalThis.process = {};
}
if (!globalThis.process.env) {
  globalThis.process.env = {};
}
if (!globalThis.process.cwd) {
  globalThis.process.cwd = () => op_php_cwd();
}

function withPrivileged(fn) {
  if (!op_php_set_privileged) {
    return fn();
  }
  op_php_set_privileged(1);
  try {
    return fn();
  } finally {
    op_php_set_privileged(0);
  }
}

function privilegedReadFileSync(path, encoding) {
  return withPrivileged(() => globalThis.fs.readFileSync(path, encoding));
}

function privilegedWriteFileSync(path, data, encoding) {
  return withPrivileged(() => globalThis.fs.writeFileSync(path, data, encoding));
}

function privilegedMkdirSync(path, options) {
  return withPrivileged(() => globalThis.fs.mkdirSync(path, options));
}

// Basic URLSearchParams implementation
globalThis.URLSearchParams = class URLSearchParams {
  constructor(init) {
    this.params = [];
    if (typeof init === 'string') {
      const pairs = init.replace(/^\?/, '').split('&');
      for (const pair of pairs) {
        if (!pair) continue;
        const idx = pair.indexOf('=');
        if (idx === -1) {
          this.params.push([decodeURIComponent(pair), '']);
        } else {
          this.params.push([
            decodeURIComponent(pair.slice(0, idx)),
            decodeURIComponent(pair.slice(idx + 1))
          ]);
        }
      }
    }
  }

  append(name, value) {
    this.params.push([String(name), String(value)]);
  }

  get(name) {
    const entry = this.params.find(([k]) => k === name);
    return entry ? entry[1] : null;
  }

  *entries() {
    for (const param of this.params) {
      yield param;
    }
  }

  toString() {
    return this.params
      .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`)
      .join('&');
  }
};

// Minimal path utilities for PHP serve mode
if (!globalThis.path) {
  globalThis.path = {};
}
if (!globalThis.path.sep) {
  globalThis.path.sep = '/';
}
if (!globalThis.path.delimiter) {
  globalThis.path.delimiter = ':';
}
if (!globalThis.path.extname) {
  globalThis.path.extname = (p) => {
    const str = String(p);
    const lastSlash = str.lastIndexOf('/');
    const lastDot = str.lastIndexOf('.');
    if (lastDot === -1 || lastDot < lastSlash) return '';
    return str.slice(lastDot);
  };
}
if (!globalThis.path.dirname) {
  globalThis.path.dirname = (p) => {
    const str = String(p);
    const lastSlash = str.lastIndexOf('/');
    if (lastSlash === -1) return '.';
    if (lastSlash === 0) return '/';
    return str.slice(0, lastSlash);
  };
}
if (!globalThis.path.basename) {
  globalThis.path.basename = (p, ext) => {
    const str = String(p);
    const lastSlash = str.lastIndexOf('/');
    let base = lastSlash === -1 ? str : str.slice(lastSlash + 1);
    if (ext && base.endsWith(ext)) {
      base = base.slice(0, -ext.length);
    }
    return base;
  };
}
if (!globalThis.path.normalize) {
  globalThis.path.normalize = (p) => {
    const str = String(p);
    const parts = str.split('/');
    const result = [];
    for (const part of parts) {
      if (part === '..') {
        if (result.length > 0 && result[result.length - 1] !== '..') {
          result.pop();
        } else {
          result.push('..');
        }
      } else if (part !== '.' && part !== '') {
        result.push(part);
      }
    }
    const normalized = result.join('/');
    return str.startsWith('/') ? '/' + normalized : normalized || '.';
  };
}
if (!globalThis.path.resolve) {
  globalThis.path.resolve = (...args) => {
    let resolved = '';
    for (let i = args.length - 1; i >= 0; i--) {
      const p = String(args[i]);
      if (!p) continue;
      if (resolved === '') {
        resolved = p;
      } else if (p.startsWith('/')) {
        resolved = p + '/' + resolved;
        break;
      } else {
        resolved = p + '/' + resolved;
      }
      if (resolved.startsWith('/')) {
        break;
      }
    }
    if (!resolved.startsWith('/')) {
      const cwd = op_php_cwd();
      resolved = cwd + '/' + resolved;
    }
    return globalThis.path.normalize(resolved);
  };
}
if (!globalThis.path.relative) {
  globalThis.path.relative = (from, to) => {
    const fromAbs = globalThis.path.resolve(from);
    const toAbs = globalThis.path.resolve(to);
    const fromParts = fromAbs.split('/').filter(Boolean);
    const toParts = toAbs.split('/').filter(Boolean);
    let shared = 0;
    while (
      shared < fromParts.length &&
      shared < toParts.length &&
      fromParts[shared] === toParts[shared]
    ) {
      shared++;
    }
    const up = fromParts.slice(shared).map(() => '..');
    const down = toParts.slice(shared);
    const combined = up.concat(down);
    return combined.length ? combined.join('/') : '.';
  };
}
if (!globalThis.path.join) {
  globalThis.path.join = (...args) => {
    const parts = args.filter(p => p && String(p) !== '');
    if (parts.length === 0) return '.';
    const joined = parts.join('/');
    return globalThis.path.normalize(joined);
  };
}

// Export nothing - this is just for side effects
export {};
