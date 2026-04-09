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
