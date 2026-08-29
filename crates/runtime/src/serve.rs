use std::path::Path as FsPath;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use std::{io, net::TcpListener};

use crate::env::init_env;
use crate::extensions::extensions_for_mode;
use crate::security::resolve_security_policy;
use core::Context;
use engine::{RuntimeEngine, RuntimeState, config as runtime_config, set_engine};
use deka_host::validation::{format_validation_error, modules::validate_module_resolution};
use notify::Watcher;
use platform::Platform;
use platform_server::ServerPlatform;
use pool::validation::{PoolWorkers, extract_pool_options};
use pool::{HandlerKey, PoolConfig};
use runtime_core::env::{flag_or_env_truthy_with, set_dev_flag_with, set_handler_path_with};
use runtime_core::modules::ensure_deka_module_root_env_with;
use runtime_core::validation::validate_deka_handler_with;
use stdio as stdio_log;
use transport::{
    DnsOptions, HttpOptions, RedisOptions, TcpOptions, UdpOptions, UnixOptions, WsOptions,
};

static WATCHER_GUARDS: OnceLock<Mutex<Vec<notify::RecommendedWatcher>>> = OnceLock::new();

pub fn serve(context: &Context) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime");

    if let Err(err) = rt.block_on(serve_async(context)) {
        stdio_log::error("serve", &err);
        std::process::exit(1);
    }
}

async fn serve_async(context: &Context) -> Result<(), String> {
    init_env();
    let platform = ServerPlatform::default();
    let resolved_security = resolve_security_policy(context)?;
    for warning in resolved_security.warnings {
        stdio_log::warn_simple(&format!("[security] {}", warning));
    }
    stdio_log::log("security", &resolved_security.summary);
    let _ = platform
        .env()
        .set("DEKA_SECURITY_POLICY", &resolved_security.policy_json);
    let _ = platform.env().set(
        "DEKA_SECURITY_NO_PROMPT",
        if resolved_security.prompt_enabled {
            "0"
        } else {
            "1"
        },
    );
    let _ = platform.env().set("DEKA_SECURITY_ENFORCE", "1");
    let env_get = |key: &str| platform.env().get(key);

    let dev_mode = dev_enabled(context);
    let watch_enabled = watch_enabled(context) || dev_mode;
    let mut env_set = |key: &str, value: &str| {
        let _ = platform.env().set(key, value);
    };
    set_dev_flag_with(dev_mode, &env_get, &mut env_set);
    let resolved = runtime_config::resolve_handler_path(&context.handler.input)
        .map_err(|err| format!("Failed to resolve handler path: {}", err))?;

    // Load neo4j/redis config from deka.json into env vars
    let config_dir = if resolved.path.is_dir() {
        &resolved.path
    } else {
        resolved.path.parent().unwrap_or(&resolved.path)
    };
    runtime_config::load_database_config(config_dir);

    let handler_path = resolved.path.to_string_lossy().to_string();
    if handler_path.to_ascii_lowercase().ends_with(".phpx") {
        return Err(format!(
            "DekaScript uses .ds only; migrate '{}' before serving it",
            handler_path
        ));
    }
    if handler_is_unsupported_script(&handler_path) {
        return Err(format!(
            "Serve mode does not execute JavaScript/TypeScript handlers: {}",
            handler_path
        ));
    }
    if matches!(resolved.mode, runtime_config::ServeMode::Php) {
        let mut env_set = |key: &str, value: &str| {
            let _ = platform.env().set(key, value);
            unsafe { std::env::set_var(key, value) };
        };
        ensure_deka_module_root_env_with(
            &handler_path,
            &|path| platform.fs().exists(path),
            &|| platform.fs().current_exe().ok(),
            &env_get,
            &mut env_set,
        );
        validate_deka_modules(&handler_path)?;
    }
    let mut env_set = |key: &str, value: &str| {
        let _ = platform.env().set(key, value);
    };
    set_handler_path_with(&handler_path, &env_get, &mut env_set);

    let handler_source = load_handler_source(&handler_path, &resolved.mode)?;

    stdio_log::log("handler", &format!("loaded {}", handler_path));
    if dev_mode {
        stdio_log::log("dev", "enabled");
    }

    let mut serve_options = pool::validation::ServeOptions::default();
    apply_cli_serve_overrides(context, &mut serve_options);

    let (server_pool_config, user_pool_config) = configure_pools(
        &handler_source,
        &handler_path,
        &serve_options,
        watch_enabled,
    );
    let server_pool_workers = server_pool_config.num_workers;

    let serve_mode = resolved.mode.clone();
    let extensions_provider = Arc::new(move || extensions_for_mode(&serve_mode));

    let runtime_cfg = runtime_config::RuntimeConfig::load();
    let engine = Arc::new(RuntimeEngine::new(
        server_pool_config,
        user_pool_config,
        &runtime_cfg,
        extensions_provider,
    ));
    let _ = set_engine(Arc::clone(&engine));

    if watch_enabled {
        if let Err(err) = start_watch(&handler_path, Arc::clone(&engine), dev_mode) {
            tracing::warn!("watch mode failed: {}", err);
        }
    }

    let handler_key = HandlerKey::new(
        FsPath::new(&handler_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&handler_path),
    );

    let handler_path_is_file = FsPath::new(&handler_path).is_file();
    // Serve mode always uses the ESM loader for DS handlers; the legacy
    // bundled-PHPX path has been removed (deka#202).
    let handler_code = String::new();
    let handler_entry = match resolved.mode {
        runtime_config::ServeMode::Php if handler_path_is_file => Some(handler_path.clone()),
        _ => None,
    };

    let perf_request_value = serde_json::json!({
        "url": "http://localhost/",
        "method": "GET",
        "headers": {},
        "body": null,
    });
    let perf_mode = perf_mode_enabled();

    let state = Arc::new(RuntimeState {
        engine: Arc::clone(&engine),
        handler_code,
        handler_entry,
        handler_key,
        dev_mode,
        perf_mode,
        perf_request_value,
    });

    spawn_archive_task(&state, engine.archive());

    serve_listeners(state, &serve_options, perf_mode, server_pool_workers).await
}

fn apply_cli_serve_overrides(
    context: &Context,
    serve_options: &mut pool::validation::ServeOptions,
) {
    if let Some(port) = context.args.params.get("--port") {
        if let Ok(value) = port.parse::<u16>() {
            serve_options.port = Some(value);
        }
    }
}

fn validate_deka_modules(handler_path: &str) -> Result<(), String> {
    validate_deka_handler_with(
        handler_path,
        &|path| {
            std::fs::read_to_string(path)
                .map_err(|err| format!("Failed to read DekaScript handler {}: {}", path, err))
        },
        &|source, path| validate_module_resolution(source, path),
        &|source, path, error| format_validation_error(source, path, error),
    )
}

fn watch_enabled(context: &Context) -> bool {
    flag_or_env_truthy_with(
        &context.args.flags,
        "--watch",
        Some("-W"),
        "DEKA_WATCH",
        &|key| std::env::var(key).ok(),
    )
}

fn dev_enabled(context: &Context) -> bool {
    flag_or_env_truthy_with(&context.args.flags, "--dev", None, "DEKA_DEV", &|key| {
        std::env::var(key).ok()
    })
}

fn perf_mode_enabled() -> bool {
    std::env::var("DEKA_PERF_MODE")
        .map(|value| value != "false" && value != "0")
        .unwrap_or(false)
}

fn load_handler_source(
    _handler_path: &str,
    _mode: &runtime_config::ServeMode,
) -> Result<String, String> {
    Ok(String::new())
}

fn configure_pools(
    handler_source: &str,
    handler_path: &str,
    serve_options: &pool::validation::ServeOptions,
    watch_enabled: bool,
) -> (PoolConfig, PoolConfig) {
    let runtime_cfg = runtime_config::RuntimeConfig::load();
    let mut server_pool_config = PoolConfig::from_env();
    let mut user_pool_config = server_pool_config.clone();

    if !handler_source.is_empty() {
        let pool_options = extract_pool_options(handler_source, handler_path);
        if let Some(workers) = pool_options.workers {
            user_pool_config.num_workers = match workers {
                PoolWorkers::Fixed(value) => {
                    if value < 1 {
                        1
                    } else {
                        value
                    }
                }
                PoolWorkers::Max => num_cpus::get(),
            };
        }
        if let Some(max) = pool_options.isolates_per_worker {
            user_pool_config.max_isolates_per_worker = max;
        }
    }

    if let Some(workers) = serve_options.workers.clone() {
        server_pool_config.num_workers = match workers {
            PoolWorkers::Fixed(value) => {
                if value < 1 {
                    1
                } else {
                    value
                }
            }
            PoolWorkers::Max => num_cpus::get(),
        };
    }
    if let Some(max) = serve_options.isolates_per_worker {
        server_pool_config.max_isolates_per_worker = max;
    }

    if let Some(enabled) = runtime_cfg.code_cache_enabled() {
        server_pool_config.enable_code_cache = enabled;
        user_pool_config.enable_code_cache = enabled;
    }

    if watch_enabled {
        server_pool_config.enable_code_cache = false;
        user_pool_config.enable_code_cache = false;
        server_pool_config.introspect_profiling = true;
        user_pool_config.introspect_profiling = true;
    }

    server_pool_config.introspect_profiling = runtime_cfg.introspect_profiling_enabled();
    user_pool_config.introspect_profiling = runtime_cfg.introspect_profiling_enabled();

    if perf_mode_enabled() {
        server_pool_config.enable_metrics = false;
        user_pool_config.enable_metrics = false;
        server_pool_config.introspect_profiling = false;
        user_pool_config.introspect_profiling = false;
    }

    (server_pool_config, user_pool_config)
}

fn build_handler_code(
    handler_path: &str,
    resolved: &runtime_config::ResolvedHandler,
) -> Result<String, String> {
    match resolved.mode {
        runtime_config::ServeMode::Php => {
            // DS handlers are loaded via the ESM loader; no pre-generated bundle
            // is needed anymore (deka#202).
            Ok(String::new())
        }
        runtime_config::ServeMode::Static => {
            let listing = resolved.config.directory_listing.unwrap_or(true);
            let static_path = std::path::Path::new(handler_path);
            let is_dir = static_path.is_dir();
            let (root, default_file) = if is_dir {
                (handler_path.to_string(), "index.html".to_string())
            } else {
                let root = static_path
                    .parent()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|| ".".to_string());
                let default_file = static_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("index.html")
                    .to_string();
                (root, default_file)
            };
            Ok(build_static_handler_code(&root, &default_file, listing))
        }
    }
}

fn handler_is_unsupported_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".ts")
        || lower.ends_with(".tsx")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
}

fn build_static_handler_code(root: &str, default_file: &str, directory_listing: bool) -> String {
    let root_json = serde_json::to_string(root).unwrap_or_else(|_| "\".\"".to_string());
    let default_json =
        serde_json::to_string(default_file).unwrap_or_else(|_| "\"index.html\"".to_string());
    let listing = if directory_listing { "true" } else { "false" };

    let template = r#"const __dekaStaticRoot = __ROOT__;
const __dekaDefaultFile = __DEFAULT__;
const __dekaDirectoryListing = __LISTING__;
const __dekaMime = {
  '.html': 'text/html; charset=utf-8',
  '.htm': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.map': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.wasm': 'application/wasm',
};

const __dekaJoin = (base, rel) => (base.endsWith('/') ? base + rel : base + '/' + rel);
const __dekaExt = (name) => {
  const i = name.lastIndexOf('.');
  return i === -1 ? '' : name.slice(i).toLowerCase();
};
const __dekaPath = globalThis.path || null;
// __dekaFs - confined to __dekaStaticRoot.
//
// Three-layer guard mirrors the @/ resolver path-traversal fix:
//   1. Reject any ".." or lone "." path segment before IO.
//   2. Resolve relative paths to absolute without symlink IO.
//   3. Assert the resolved path starts with __dekaStaticRoot.
//
// All methods fail-silently (return null / false / void) on out-of-root
// access, matching the existing catch-all behaviour used by callers.
const __dekaFs = (() => {
  const _raw = globalThis.fs || null;
  if (!_raw) return null;
  const _rootSlash = __dekaStaticRoot.endsWith('/')
    ? __dekaStaticRoot
    : __dekaStaticRoot + '/';
  function _allow(p) {
    const s = String(p || '');
    const segs = s.replace(/\\/g, '/').split('/');
    for (const seg of segs) {
      if (seg === '..') return false;
    }
    // Resolve to absolute.
    let abs = s.startsWith('/') ? s : _rootSlash + s;
    // Collapse redundant segments (textual pre-canonicalize step).
    const parts = abs.split('/');
    const out = [];
    for (const seg of parts) {
      if (seg === '' || seg === '.') continue;
      if (seg === '..') { out.pop(); continue; }
      out.push(seg);
    }
    abs = '/' + out.join('/');
    // Symlink resolution: use op_php_canonicalize so the OS resolves all
    // symlinks before the prefix check. Returns null for non-existent paths;
    // fall through to the textual path so new-file writes inside the root
    // still work.
    let canon = null;
    try {
      if (typeof op_php_canonicalize === 'function') canon = op_php_canonicalize(abs);
    } catch (_) {}
    const check = (typeof canon === 'string' && canon) ? canon : abs;
    const checkSlash = check.endsWith('/') ? check : check + '/';
    return checkSlash.startsWith(_rootSlash) || check === _rootSlash.slice(0, -1);
  }
  return {
    statSync: (p) => {
      if (!_allow(p)) return null;
      try { if (typeof _raw.statSync === 'function') return _raw.statSync(p); } catch (_) {}
      return null;
    },
    readFileSync: (p, enc) => {
      if (!_allow(p)) return null;
      try { if (typeof _raw.readFileSync === 'function') return _raw.readFileSync(p, enc); } catch (_) {}
      return null;
    },
    readdirSync: (p, opts) => {
      if (!_allow(p)) return null;
      try { if (typeof _raw.readdirSync === 'function') return _raw.readdirSync(p, opts); } catch (_) {}
      return null;
    },
    writeFileSync: (p, data, enc) => {
      if (!_allow(p)) return;
      try { if (typeof _raw.writeFileSync === 'function') _raw.writeFileSync(p, data, enc); } catch (_) {}
    },
    mkdirSync: (p, opts) => {
      if (!_allow(p)) return;
      try { if (typeof _raw.mkdirSync === 'function') _raw.mkdirSync(p, opts); } catch (_) {}
    },
    appendFileSync: (p, data) => {
      if (!_allow(p)) return;
      try { if (typeof _raw.appendFileSync === 'function') _raw.appendFileSync(p, data); } catch (_) {}
    },
    existsSync: (p) => {
      if (!_allow(p)) return false;
      try { if (typeof _raw.existsSync === 'function') return _raw.existsSync(p); } catch (_) {}
      return false;
    },
  };
})();
const __dekaPathJoin = (...parts) => {
  if (__dekaPath && typeof __dekaPath.join === 'function') return __dekaPath.join(...parts);
  return parts.filter(Boolean).join('/');
};
const __dekaSafeRel = (pathname) => {
  let p = pathname || '/';
  try { p = decodeURIComponent(p); } catch (_) { return null; }
  if (p === '/' || p === '') p = '/' + __dekaDefaultFile;
  if (p.startsWith('/')) p = p.slice(1);
  if (!p || p.split('/').includes('..') || p.includes('\0')) return null;
  return p;
};
const __dekaText = (status, message) => new Response(message + '\n', {
  status,
  headers: { 'content-type': 'text/plain; charset=utf-8' },
});
const __dekaHtml = (status, body) => new Response(body, {
  status,
  headers: { 'content-type': 'text/html; charset=utf-8' },
});
const __dekaStat = (target) => {
  try {
    if (__dekaFs && typeof __dekaFs.statSync === 'function') return __dekaFs.statSync(target);
  } catch (_err) {}
  return null;
};
const __dekaReadFile = (target) => {
  try {
    if (__dekaFs && typeof __dekaFs.readFileSync === 'function') return __dekaFs.readFileSync(target);
  } catch (_err) {}
  return null;
};
const __dekaReadDir = (target) => {
  try {
    if (__dekaFs && typeof __dekaFs.readdirSync === 'function') return __dekaFs.readdirSync(target, { withFileTypes: true });
  } catch (_err) {}
  return null;
};
const __dekaIsDirectory = (stat) => {
  if (!stat) return false;
  if (typeof stat.isDirectory === 'function') return !!stat.isDirectory();
  return !!stat.isDirectory;
};
const __dekaEntryName = (entry) => {
  if (!entry) return '';
  if (typeof entry === 'string') return entry;
  return String(entry.name || '');
};
const __dekaEntryIsDir = (entry) => {
  if (!entry) return false;
  if (typeof entry.isDirectory === 'function') return !!entry.isDirectory();
  if (typeof entry.isDirectory === 'boolean') return entry.isDirectory;
  if (typeof entry.is_dir === 'boolean') return entry.is_dir;
  if (typeof entry.kind === 'string') return entry.kind === 'directory';
  return false;
};
const __dekaObjectMapStringToBytes = (text) => {
  if (typeof text !== 'string') return null;
  const t = text.trim();
  if (!t.startsWith('{') || !t.includes('"0"')) return null;

  // Fast path for huge object-map strings like {"0":0,"1":97,...}
  try {
    const re = /"(\d+)":(-?\d+)/g;
    let match = null;
    let seen = 0;
    let max = -1;
    const tmp = [];
    while ((match = re.exec(t)) !== null) {
      const idx = Number(match[1]);
      const val = Number(match[2]);
      if (!Number.isFinite(idx) || idx < 0) continue;
      tmp[idx] = Number.isFinite(val) ? (val & 255) : 0;
      if (idx > max) max = idx;
      seen++;
    }
    if (seen > 0 && max >= 0) {
      const out = new Uint8Array(max + 1);
      for (let i = 0; i <= max; i++) out[i] = tmp[i] || 0;
      return out;
    }
  } catch (_err) {}

  try {
    const parsed = JSON.parse(t);
    const keys = Object.keys(parsed);
    if (keys.length === 0 || !keys.every((k) => /^\d+$/.test(k))) return null;
    const bytes = keys
      .sort((a, b) => Number(a) - Number(b))
      .map((k) => Number(parsed[k]) || 0);
    return new Uint8Array(bytes);
  } catch (_err) {
    return null;
  }
};

const __dekaToBytes = (value) => {
  if (value == null) return null;
  if (typeof Uint8Array !== 'undefined' && value instanceof Uint8Array) return value;
  if (typeof ArrayBuffer !== 'undefined' && value instanceof ArrayBuffer) return new Uint8Array(value);
  if (typeof value === 'string') return new TextEncoder().encode(value);
  if (typeof value === 'object') {
    const keys = Object.keys(value);
    if (keys.length > 0 && keys.every((k) => /^\d+$/.test(k))) {
      const bytes = keys
        .sort((a, b) => Number(a) - Number(b))
        .map((k) => Number(value[k]) || 0);
      return new Uint8Array(bytes);
    }
  }
  return null;
};
const __dekaBody = (value, mime) => {
  const mimeText = String(mime || '');
  const isTextLike = mimeText.startsWith('text/')
    || mimeText.includes('javascript')
    || mimeText.includes('json')
    || mimeText.includes('svg+xml');
  const bytes = __dekaToBytes(value);
  if (isTextLike) {
    if (typeof value === 'string') return value;
    if (bytes) {
      try { return new TextDecoder().decode(bytes); } catch (_err) {}
    }
    return value == null ? '' : String(value);
  }
  if (bytes) {
    if (typeof value === 'string') {
      const remapped = __dekaObjectMapStringToBytes(value);
      if (remapped) return remapped;
    }
    return bytes;
  }
  if (typeof value === 'string') {
    const remapped = __dekaObjectMapStringToBytes(value);
    if (remapped) return remapped;
    return value;
  }
  return value == null ? '' : String(value);
};

const app = {
  async fetch(req) {
    const url = new URL(req.url);
    const rel = __dekaSafeRel(url.pathname);
    if (!rel) return __dekaText(400, 'Bad Request');

    let target = __dekaJoin(__dekaStaticRoot, rel);
    const stat = __dekaStat(target);
    if (__dekaIsDirectory(stat)) {
      const indexTarget = __dekaPathJoin(target, 'index.html');
      const indexBytes = __dekaReadFile(indexTarget);
      if (indexBytes != null) {
        return new Response(__dekaBody(indexBytes, __dekaMime['.html']), {
          status: 200,
          headers: { 'content-type': __dekaMime['.html'] },
        });
      }
      if (!__dekaDirectoryListing) return __dekaText(403, 'Directory listing disabled');
      const entries = __dekaReadDir(target);
      if (!Array.isArray(entries)) return __dekaText(404, 'Not Found');
      const links = entries.map((entry) => {
        const name = __dekaEntryName(entry);
        if (!name) return '';
        const suffix = __dekaEntryIsDir(entry) ? '/' : '';
        const href = (url.pathname.endsWith('/') ? url.pathname : url.pathname + '/') + name + suffix;
        return `<li><a href="${href}">${name}${suffix}</a></li>`;
      }).join('');
      return __dekaHtml(200, `<h1>Index of ${url.pathname}</h1><ul>${links}</ul>`);
    }

    const bytes = __dekaReadFile(target);
    if (bytes == null) return __dekaText(404, 'Not Found');
    const mime = __dekaMime[__dekaExt(target)] || 'application/octet-stream';
    return new Response(__dekaBody(bytes, mime), {
      status: 200,
      headers: { 'content-type': mime },
    });
  }
};

globalThis.app = app;
"#;

    template
        .replace("__ROOT__", &root_json)
        .replace("__DEFAULT__", &default_json)
        .replace("__LISTING__", listing)
}

async fn serve_listeners(
    state: Arc<RuntimeState>,
    serve_options: &pool::validation::ServeOptions,
    perf_mode: bool,
    server_pool_workers: usize,
) -> Result<(), String> {
    if let Some(unix) = serve_options
        .unix
        .clone()
        .or_else(|| std::env::var("DEKA_UNIX").ok())
    {
        let label = if unix.starts_with('\0') {
            format!("unix:@{}", unix.trim_start_matches('\0'))
        } else {
            format!("unix:{}", unix)
        };
        stdio_log::log("listen", &label);
        return transport::serve(
            state,
            transport::ListenConfig::Unix(UnixOptions { path: unix }),
        )
        .await;
    }

    if let Some(addr) = serve_options
        .tcp
        .clone()
        .or_else(|| std::env::var("DEKA_TCP").ok())
    {
        stdio_log::log("listen", &format!("tcp://{}", addr));
        return transport::serve(state, transport::ListenConfig::Tcp(TcpOptions { addr })).await;
    }

    if let Some(addr) = serve_options
        .udp
        .clone()
        .or_else(|| std::env::var("DEKA_UDP").ok())
    {
        stdio_log::log("listen", &format!("udp://{}", addr));
        return transport::serve(state, transport::ListenConfig::Udp(UdpOptions { addr })).await;
    }

    if let Some(addr) = serve_options
        .dns
        .clone()
        .or_else(|| std::env::var("DEKA_DNS").ok())
    {
        stdio_log::log("listen", &format!("dns://{}", addr));
        return transport::serve(state, transport::ListenConfig::Dns(DnsOptions { addr })).await;
    }

    if let Some(port) = serve_options.ws.or_else(|| {
        std::env::var("DEKA_WS")
            .ok()
            .and_then(|value| value.parse().ok())
    }) {
        stdio_log::log("listen", &format!("ws://localhost:{}", port));
        return transport::serve(state, transport::ListenConfig::Ws(WsOptions { port })).await;
    }

    if let Some(addr) = serve_options
        .redis
        .clone()
        .or_else(|| std::env::var("DEKA_REDIS").ok())
    {
        stdio_log::log("listen", &format!("redis://{}", addr));
        return transport::serve(state, transport::ListenConfig::Redis(RedisOptions { addr }))
            .await;
    }

    let port = serve_options
        .port
        .or_else(|| std::env::var("PORT").ok().and_then(|p| p.parse().ok()))
        .unwrap_or(8530);
    ensure_http_port_available(port)?;
    let listeners = server_pool_workers.max(1);

    stdio_log::log("listen", &format!("http://localhost:{}", port));
    transport::serve(
        state,
        transport::ListenConfig::Http(HttpOptions {
            port,
            listeners,
            perf_mode,
        }),
    )
    .await?;
    Ok(())
}

fn ensure_http_port_available(port: u16) -> Result<(), String> {
    match TcpListener::bind(("0.0.0.0", port)) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(err) => {
            let message = match err.kind() {
                io::ErrorKind::AddrInUse => format!(
                    "port {} is already in use. `deka serve` refuses to share ports between processes; stop the existing server or pass --port <n>.",
                    port
                ),
                _ => format!("failed to verify HTTP port {} availability: {}", port, err),
            };
            Err(message)
        }
    }
}

fn spawn_archive_task(state: &Arc<RuntimeState>, archive: Option<engine::IntrospectArchive>) {
    let Some(archive) = archive else {
        return;
    };
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let cutoff_ms = now_millis().saturating_sub(60_000);
            let traces = state.engine.drain_request_history_before(cutoff_ms).await;
            if traces.is_empty() {
                continue;
            }
            let archive = archive.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = archive.record_traces(&traces);
            })
            .await;
        }
    });
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

fn start_watch(
    handler_path: &str,
    engine: Arc<RuntimeEngine>,
    dev_mode: bool,
) -> Result<(), String> {
    let path = FsPath::new(handler_path);
    let watch_root = path.parent().unwrap_or_else(|| FsPath::new("."));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<notify::Result<notify::Event>>();

    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .map_err(|err| err.to_string())?;

    watcher
        .watch(watch_root, notify::RecursiveMode::Recursive)
        .map_err(|err| err.to_string())?;

    // Keep watcher alive for process lifetime; dropping it stops event delivery.
    if let Ok(mut guards) = WATCHER_GUARDS.get_or_init(|| Mutex::new(Vec::new())).lock() {
        guards.push(watcher);
    }

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                Ok(event) => {
                    let mut changed: Vec<String> = Vec::new();
                    for path in &event.paths {
                        if should_ignore_watch_path(path) {
                            continue;
                        }
                        let normalized = path.to_string_lossy().replace('\\', "/");
                        if !changed.iter().any(|existing| existing == &normalized) {
                            changed.push(normalized);
                        }
                    }
                    if changed.is_empty() {
                        continue;
                    }

                    if dev_mode {
                        stdio_log::log("hmr", &format!("changed {}", changed.join(", ")));
                        transport::notify_hmr_changed(&changed);
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    let evicted = engine.pool().evict_all().await;
                    if evicted > 0 {
                        stdio_log::log("watch", &format!("evicted {}", evicted));
                    }
                }
                Err(err) => {
                    tracing::warn!("watch error: {}", err);
                }
            }
        }
    });

    Ok(())
}

fn should_ignore_watch_path(path: &FsPath) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        return true;
    }

    if normalized.ends_with("/deka.lock") {
        return true;
    }

    // Generated/transient paths that should not trigger HMR loops.
    for needle in [
        "/.cache/",
        "/php_modules/.cache/",
        "/node_modules/.cache/",
        "/target/",
        "/.git/",
    ] {
        if normalized.contains(needle) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::build_static_handler_code;
    use super::ensure_http_port_available;
    use super::flag_or_env_truthy_with;
    use super::serve_async;
    use core::{Args, EnvContext, HandlerContext};
    use runtime_core::env::is_truthy;
    use std::collections::HashMap;
    use std::fs;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    // DEKA_SECURITY_POLICY, ISOLATE_WORKERS, and the DEKA_*_ENV probe vars
    // set by the real-topology serve tests below are all process-global
    // (std::env::set_var). Rust runs `#[test]`s concurrently by default, so
    // without serialization these tests race on the same env vars and
    // intermittently read each other's policy/values (tana#913 QA flake).
    // Mirrors the TEST_ENV_LOCK precedent in js_pipeline.rs for
    // DEKA_MODULE_ROOT-mutating tests.
    static SERVE_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Verify the static handler template contains the __dekaFs confinement
    /// wrapper.  We check for the key guard identifiers that must be present
    /// for path-prefix enforcement.
    #[test]
    fn static_handler_dekafs_is_path_confined() {
        let code = build_static_handler_code("/srv/static", "index.html", false);
        // Wrapper must be an IIFE (not a bare reference to globalThis.fs).
        assert!(
            code.contains("const __dekaFs = (() => {"),
            "expected __dekaFs to be an IIFE wrapper"
        );
        // Must reference __dekaStaticRoot as the confinement boundary.
        assert!(
            code.contains("__dekaStaticRoot"),
            "expected __dekaFs wrapper to reference __dekaStaticRoot"
        );
        // The _allow guard must be present.
        assert!(
            code.contains("function _allow("),
            "expected path-allow guard in __dekaFs wrapper"
        );
        // Must reject '..' segments explicitly.
        assert!(
            code.contains("if (seg === '..') return false"),
            "expected '..' rejection in _allow guard"
        );
        // The raw globalThis.fs must NOT be directly assigned to __dekaFs.
        assert!(
            !code.contains("const __dekaFs = globalThis.fs"),
            "expected __dekaFs NOT to be a bare globalThis.fs alias"
        );
    }

    #[test]
    fn truthy_parser_matches_expected_values() {
        assert!(is_truthy("1"));
        assert!(is_truthy("true"));
        assert!(is_truthy("yes"));
        assert!(is_truthy("on"));
        assert!(!is_truthy("0"));
        assert!(!is_truthy("false"));
        assert!(!is_truthy("off"));
    }

    #[test]
    fn flag_overrides_env_for_watch_or_dev() {
        let mut flags = HashMap::new();
        flags.insert("--dev".to_string(), true);
        assert!(flag_or_env_truthy_with(
            &flags,
            "--dev",
            None,
            "DEKA_DEV",
            &|_| None,
        ));

        let mut watch_flags = HashMap::new();
        watch_flags.insert("-W".to_string(), true);
        assert!(flag_or_env_truthy_with(
            &watch_flags,
            "--watch",
            Some("-W"),
            "DEKA_WATCH",
            &|_| None,
        ));
    }

    #[test]
    fn rejects_occupied_http_port() {
        let listener = TcpListener::bind(("0.0.0.0", 0)).expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        let err = ensure_http_port_available(port).expect_err("port should be rejected");
        assert!(err.contains("already in use"), "unexpected error: {}", err);
    }




    /// Blocker 1 regression: __dekaStat/__dekaReadFile/__dekaReadDir must NOT
    /// contain Deno.* fallback branches.  If a tenant can shadow globalThis.fs
    /// to null the helpers must fail-closed (return null) rather than falling
    /// through to raw Deno ops that bypass the __dekaFs confinement wrapper.
    #[test]
    fn static_handler_no_deno_fallbacks_in_helpers() {
        let code = build_static_handler_code("/srv/static", "index.html", false);
        assert!(
            !code.contains("Deno.statSync"),
            "__dekaStat must not contain Deno.statSync fallback"
        );
        assert!(
            !code.contains("Deno.readFileSync"),
            "__dekaReadFile must not contain Deno.readFileSync fallback"
        );
        assert!(
            !code.contains("Deno.readDirSync"),
            "__dekaReadDir must not contain Deno.readDirSync fallback"
        );
    }

    /// Blocker 2 companion: verify the static handler _allow guard now calls
    /// op_php_canonicalize before the prefix check.
    #[test]
    fn static_handler_allow_uses_canonicalize() {
        let code = build_static_handler_code("/srv/static", "index.html", false);
        assert!(
            code.contains("op_php_canonicalize"),
            "_allow guard in __dekaFs must call op_php_canonicalize for symlink resolution"
        );
    }

    /// Blocker 2: symlink containment.
    ///
    /// Tenant A's root contains a symlink pointing at tenant B's secret file.
    /// std::fs::canonicalize resolves the symlink; the prefix check then sees
    /// the real path (outside tenant A's root) and rejects it.
    ///
    /// This test validates the Rust-layer logic that backs op_php_canonicalize
    /// and confirms it is exactly what the JS guard calls.
    #[test]
    fn canonicalize_catches_symlink_escape() {
        use std::fs;
        use std::os::unix::fs as unix_fs;

        // Create tenant roots. Canonicalize immediately so the macOS
        // /var -> /private/var symlink does not confuse starts_with.
        let tmp = std::env::temp_dir();
        let root_a_pre = tmp.join(format!("deka_test_tenant_a_{}", std::process::id()));
        let root_b_pre = tmp.join(format!("deka_test_tenant_b_{}", std::process::id()));
        fs::create_dir_all(&root_a_pre).unwrap();
        fs::create_dir_all(&root_b_pre).unwrap();
        let root_a = fs::canonicalize(&root_a_pre).expect("canonicalize root_a");
        let root_b = fs::canonicalize(&root_b_pre).expect("canonicalize root_b");

        // B has a secret file.
        let secret_b = root_b.join("secret.txt");
        fs::write(&secret_b, "B's secret").unwrap();

        // Symlink inside tenant A pointing at B's secret.
        let symlink_in_a = root_a.join("peek_b");
        unix_fs::symlink(&secret_b, &symlink_in_a).unwrap();

        // The symlink path PASSES a naive starts_with check (textual).
        assert!(
            symlink_in_a.starts_with(&root_a),
            "sanity: symlink IS inside tenant A's root textually"
        );

        // std::fs::canonicalize follows the symlink and resolves to B's path.
        let canonical = fs::canonicalize(&symlink_in_a)
            .expect("canonicalize must succeed: symlink target exists");
        assert!(
            !canonical.starts_with(&root_a),
            "canonicalized path must NOT be inside tenant A's root: got {}",
            canonical.display()
        );
        assert!(
            canonical.starts_with(&root_b),
            "canonicalized path must point inside tenant B's root: got {}",
            canonical.display()
        );

        // Verify a legitimate file inside A stays inside A after canonicalize.
        let real_file_a = root_a.join("real.txt");
        fs::write(&real_file_a, "A's data").unwrap();
        let canon_real = fs::canonicalize(&real_file_a).expect("canonicalize real file");
        assert!(
            canon_real.starts_with(&root_a),
            "real file inside A must canonicalize to within A"
        );

        // Cleanup.
        let _ = fs::remove_dir_all(&root_a);
        let _ = fs::remove_dir_all(&root_b);
    }

}
