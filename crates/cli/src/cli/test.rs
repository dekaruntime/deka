use core::{CommandSpec, Context, FlagSpec, ParamSpec, Registry};
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use stdio;

const COMMAND: CommandSpec = CommandSpec {
    name: "test",
    category: "runtime",
    summary: "run tests in deka runtime",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

const TEST_FILE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--no-rust",
        aliases: &[],
        description: "skip cargo test if Cargo.toml is present",
    });
    registry.add_param(ParamSpec {
        name: "--test-name-pattern",
        description: "only run tests with names matching the pattern",
    });
    registry.add_param(ParamSpec {
        name: "-t",
        description: "only run tests with names matching the pattern",
    });
    registry.add_param(ParamSpec {
        name: "--preload",
        description: "comma-separated list of preload modules",
    });
}

pub fn cmd(context: &Context) {
    if let Err(message) = run_tests(context) {
        stdio::error("test", &message);
        std::process::exit(1);
    }
}

fn run_tests(context: &Context) -> Result<(), String> {
    let filters = context.args.positionals.clone();
    let port = context
        .args
        .params
        .get("--port")
        .cloned()
        .unwrap_or_else(|| "8530".to_string());
    let test_name_pattern = context
        .args
        .params
        .get("--test-name-pattern")
        .or_else(|| context.args.params.get("-t"))
        .cloned();

    if !context.args.flags.contains_key("--no-rust") {
        run_rust_tests()?;
    }

    let cwd = context.env.cwd.clone();
    let preloads = normalize_paths(&cwd, parse_preloads(context.args.params.get("--preload")));
    let resolved_files = resolve_test_files(&cwd, &filters)?;
    if resolved_files.is_empty() {
        stdio::log("test", "no deka tests found");
        return Ok(());
    }

    let handler_source = build_handler_source(&resolved_files, &preloads, test_name_pattern);
    let temp_dir = create_temp_dir()?;
    let handler_path = temp_dir.join("handler.ts");
    fs::write(&handler_path, handler_source)
        .map_err(|err| format!("failed to write test handler: {}", err))?;

    stdio::log(
        "test",
        &format!("[runtime] {} test file(s)", resolved_files.len()),
    );
    for file in &resolved_files {
        let rel = file.strip_prefix(&cwd).unwrap_or(file).to_string_lossy();
        stdio::log("test", &format!("[deka:{}]", rel));
    }

    let mut child = spawn_runtime(context, &handler_path, &port)?;
    let mut ok = true;
    let mut response_body = String::new();

    for _ in 0..20 {
        if let Some(result) = fetch_with_timeout(&port, Duration::from_secs(10)) {
            let (status, body) = result?;
            response_body = body;
            ok = status >= 200 && status < 300;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    if response_body.is_empty() {
        ok = false;
        stdio::error("test", "runtime failed to start");
    } else if ok {
        let trimmed = response_body.trim();
        stdio::log("test", if trimmed.is_empty() { "ok" } else { trimmed });
    } else {
        let trimmed = response_body.trim();
        stdio::error(
            "test",
            if trimmed.is_empty() {
                "tests failed"
            } else {
                trimmed
            },
        );
    }

    stop_child(&mut child);
    if std::env::var("DEKA_TEST_KEEP").ok().as_deref() != Some("1") {
        let _ = fs::remove_dir_all(&temp_dir);
    }

    if ok {
        Ok(())
    } else {
        Err("tests failed".to_string())
    }
}

fn run_rust_tests() -> Result<(), String> {
    if !Path::new("Cargo.toml").exists() {
        return Ok(());
    }
    stdio::log("test", "[rust] running cargo test");
    let status = Command::new("cargo")
        .arg("test")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|err| format!("failed to run cargo test: {}", err))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo test failed (code {})",
            status.code().unwrap_or(1)
        ))
    }
}

fn parse_preloads(value: Option<&String>) -> Vec<PathBuf> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| PathBuf::from(part))
        .collect()
}

fn normalize_paths(cwd: &Path, paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .collect()
}

fn resolve_test_files(cwd: &Path, filters: &[String]) -> Result<Vec<PathBuf>, String> {
    let all_tests = discover_all_tests(cwd)?;
    if filters.is_empty() {
        return Ok(all_tests);
    }

    let mut resolved = Vec::new();
    let mut substring_filters = Vec::new();
    for entry in filters {
        let candidate = cwd.join(entry);
        if candidate.exists() {
            let mut matches = scan_for_tests(&candidate)?;
            if matches.is_empty() && candidate.is_file() {
                matches.push(candidate);
            }
            resolved.extend(matches);
            continue;
        }
        substring_filters.push(entry.as_str());
    }

    if !substring_filters.is_empty() {
        for file in &all_tests {
            let haystack = file.to_string_lossy();
            if substring_filters
                .iter()
                .any(|filter| haystack.contains(filter))
            {
                resolved.push(file.clone());
            }
        }
    }

    Ok(unique_paths(resolved))
}

fn discover_all_tests(cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let tests_dir = cwd.join("tests");
    let test_dir = cwd.join("test");
    if tests_dir.exists() || test_dir.exists() {
        let mut results = Vec::new();
        if tests_dir.exists() {
            results.extend(scan_for_tests(&tests_dir)?);
        }
        if test_dir.exists() {
            results.extend(scan_for_tests(&test_dir)?);
        }
        return Ok(unique_paths(results));
    }
    scan_for_tests(cwd)
}

fn scan_for_tests(root: &Path) -> Result<Vec<PathBuf>, String> {
    if root.is_file() {
        return Ok(if is_test_file(root) {
            vec![root.to_path_buf()]
        } else {
            Vec::new()
        });
    }

    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                return Err(format!("failed to read {}: {}", dir.display(), err));
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                if should_skip_dir(&path) {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() && is_test_file(&path) {
                results.push(path);
            }
        }
    }
    Ok(results)
}

fn should_skip_dir(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    matches!(name, "target" | "dist" | ".git")
}

fn is_test_file(path: &Path) -> bool {
    let file_name = match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name.to_ascii_lowercase(),
        None => return false,
    };
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    if stem == "test" && TEST_FILE_EXTENSIONS.contains(&ext) {
        return true;
    }

    file_name.contains(".test.")
        || file_name.contains(".spec.")
        || file_name.contains("_test.")
        || file_name.contains("_spec.")
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for path in paths {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            output.push(path);
        }
    }
    output
}

fn build_handler_source(
    files: &[PathBuf],
    preloads: &[PathBuf],
    test_name_pattern: Option<String>,
) -> String {
    let files_json = serde_json::to_string(
        &files
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string());
    let preloads_json = serde_json::to_string(
        &preloads
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string());
    let pattern_json = serde_json::to_string(&test_name_pattern.unwrap_or_default())
        .unwrap_or_else(|_| "\"\"".to_string());

    format!(
        r#"
import {{ serve }} from "deka";

type TestFn = () => unknown | Promise<unknown>;

type TestCase = {{
  name: string;
  fn: TestFn;
  suite: Suite;
}};

type Suite = {{
  name: string;
  parent: Suite | null;
  suites: Suite[];
  tests: TestCase[];
  beforeAll: TestFn[];
  afterAll: TestFn[];
  beforeEach: TestFn[];
  afterEach: TestFn[];
}};

type TestResult = {{
  name: string;
  fullName: string;
  status: "passed" | "failed";
  durationMs: number;
  error?: string;
}};

type TestSummary = {{
  total: number;
  passed: number;
  failed: number;
  results: TestResult[];
}};

let rootSuite = createSuite("root", null);
let currentSuite = rootSuite;
const namePattern = {pattern_json} ? new RegExp({pattern_json}) : null;

function createSuite(name: string, parent: Suite | null): Suite {{
  return {{
    name,
    parent,
    suites: [],
    tests: [],
    beforeAll: [],
    afterAll: [],
    beforeEach: [],
    afterEach: [],
  }};
}}

function resetState() {{
  rootSuite = createSuite("root", null);
  currentSuite = rootSuite;
}}

function pushSuite(name: string, fn: TestFn) {{
  const suite = createSuite(name, currentSuite);
  currentSuite.suites.push(suite);
  currentSuite = suite;
  const result = fn();
  if (result && typeof (result as Promise<unknown>).then === "function") {{
    throw new Error("describe(\\\"" + name + "\\\") must be synchronous");
  }}
  currentSuite = suite.parent || rootSuite;
}}

function addTest(name: string, fn: TestFn) {{
  currentSuite.tests.push({{ name, fn, suite: currentSuite }});
}}

function addHook(kind: "beforeAll" | "afterAll" | "beforeEach" | "afterEach", fn: TestFn) {{
  currentSuite[kind].push(fn);
}}

function createExpect(received: unknown) {{
  return {{
    toBe(expected: unknown) {{
      if (!Object.is(received, expected)) {{
        throw new Error("Expected " + format(received) + " to be " + format(expected));
      }}
    }},
    toEqual(expected: unknown) {{
      if (!deepEqual(received, expected)) {{
        throw new Error("Expected " + format(received) + " to equal " + format(expected));
      }}
    }},
    toBeTruthy() {{
      if (!received) {{
        throw new Error("Expected " + format(received) + " to be truthy");
      }}
    }},
    toBeFalsy() {{
      if (received) {{
        throw new Error("Expected " + format(received) + " to be falsy");
      }}
    }},
    toBeDefined() {{
      if (received === undefined) {{
        throw new Error("Expected value to be defined");
      }}
    }},
    toBeNull() {{
      if (received !== null) {{
        throw new Error("Expected " + format(received) + " to be null");
      }}
    }},
    toContain(expected: unknown) {{
      if (!Array.isArray(received) && typeof received !== "string") {{
        throw new Error("Expected " + format(received) + " to contain " + format(expected));
      }}
      if ((received as any).includes(expected) !== true) {{
        throw new Error("Expected " + format(received) + " to contain " + format(expected));
      }}
    }},
    toBeGreaterThan(expected: number) {{
      if (typeof received !== "number" || typeof expected !== "number") {{
        throw new Error("Expected " + format(received) + " to be a number");
      }}
      if (!(received > expected)) {{
        throw new Error("Expected " + format(received) + " to be greater than " + format(expected));
      }}
    }},
    toBeInstanceOf(expected: Function) {{
      if (!(received instanceof (expected as any))) {{
        throw new Error("Expected " + format(received) + " to be instance of " + format(expected));
      }}
    }},
    toMatch(expected: RegExp | string) {{
      const value = String(received);
      if (expected instanceof RegExp) {{
        if (!expected.test(value)) {{
          throw new Error("Expected " + format(received) + " to match " + expected);
        }}
        return;
      }}
      if (!value.includes(expected)) {{
        throw new Error("Expected " + format(received) + " to match " + format(expected));
      }}
    }},
    toThrow(expected?: RegExp | string) {{
      if (typeof received !== "function") {{
        throw new Error("Expected " + format(received) + " to be a function");
      }}
      let thrown: unknown;
      try {{
        (received as () => void)();
      }} catch (error) {{
        thrown = error;
      }}
      if (!thrown) {{
        throw new Error("Expected function to throw");
      }}
      if (expected) {{
        const message = (thrown as Error)?.message ?? String(thrown);
        if (expected instanceof RegExp) {{
          if (!expected.test(message)) {{
            throw new Error("Expected error message to match " + expected);
          }}
        }} else if (!message.includes(expected)) {{
          throw new Error("Expected error message to include " + format(expected));
        }}
      }}
    }},
  }};
}}

function getBeforeEachHooks(suite: Suite) {{
  const hooks: TestFn[] = [];
  let current: Suite | null = suite;
  const chain: Suite[] = [];
  while (current) {{
    chain.push(current);
    current = current.parent;
  }}
  for (const item of chain.reverse()) {{
    hooks.push(...item.beforeEach);
  }}
  return hooks;
}}

function getAfterEachHooks(suite: Suite) {{
  const hooks: TestFn[] = [];
  let current: Suite | null = suite;
  while (current) {{
    hooks.push(...current.afterEach);
    current = current.parent;
  }}
  return hooks;
}}

function getFullName(test: TestCase) {{
  const names: string[] = [];
  let current: Suite | null = test.suite;
  while (current && current.parent) {{
    names.push(current.name);
    current = current.parent;
  }}
  names.reverse();
  names.push(test.name);
  return names.join(" > ");
}}

async function runSuite(suite: Suite, summary: TestSummary) {{
  for (const hook of suite.beforeAll) {{
    await hook();
  }}

  for (const test of suite.tests) {{
    const fullName = getFullName(test);
    if (namePattern && !namePattern.test(fullName)) {{
      continue;
    }}
    const start = Date.now();
    try {{
      for (const hook of getBeforeEachHooks(test.suite)) {{
        await hook();
      }}
      await test.fn();
      for (const hook of getAfterEachHooks(test.suite)) {{
        await hook();
      }}
      const durationMs = Date.now() - start;
      summary.passed += 1;
      summary.results.push({{
        name: test.name,
        fullName,
        status: "passed",
        durationMs,
      }});
    }} catch (error) {{
      const durationMs = Date.now() - start;
      summary.failed += 1;
      summary.results.push({{
        name: test.name,
        fullName,
        status: "failed",
        durationMs,
        error: formatError(error),
      }});
    }}
    summary.total += 1;
  }}

  for (const child of suite.suites) {{
    await runSuite(child, summary);
  }}

  for (const hook of suite.afterAll) {{
    await hook();
  }}
}}

async function runTests(files: string[]): Promise<TestSummary> {{
  resetState();
  const preloads = {preloads_json};
  for (const preload of preloads) {{
    if (typeof (globalThis as any).__dekaLoadModule !== "function") {{
      throw new Error("Module loader is not installed");
    }}
    (globalThis as any).__dekaLoadModule(preload);
  }}
  for (const file of files) {{
    if (typeof (globalThis as any).__dekaLoadModule !== "function") {{
      throw new Error("Module loader is not installed");
    }}
    (globalThis as any).__dekaLoadModule(file);
  }}

  const summary: TestSummary = {{ total: 0, passed: 0, failed: 0, results: [] }};
  await runSuite(rootSuite, summary);
  printSummary(summary);
  return summary;
}}

function printSummary(summary: TestSummary) {{
  for (const result of summary.results) {{
    if (result.status === "passed") {{
      console.log("[pass] " + result.fullName + " (" + result.durationMs + "ms)");
    }} else {{
      console.log("[fail] " + result.fullName + " (" + result.durationMs + "ms)");
      if (result.error) {{
        console.log(result.error);
      }}
    }}
  }}
  console.log(
    "[test] total=" + summary.total + " passed=" + summary.passed + " failed=" + summary.failed
  );
}}

function format(value: unknown) {{
  if (typeof value === "string") return JSON.stringify(value);
  if (typeof value === "function") return "[Function]";
  if (value instanceof Error) return value.stack || value.message;
  try {{
    return JSON.stringify(value);
  }} catch {{
    return String(value);
  }}
}}

function formatError(error: unknown) {{
  if (error instanceof Error) {{
    return error.stack || error.message;
  }}
  return String(error);
}}

function deepEqual(a: unknown, b: unknown): boolean {{
  if (Object.is(a, b)) return true;
  if (typeof a !== typeof b) return false;
  if (a && b && typeof a === "object") {{
    const aObj = a as Record<string, unknown>;
    const bObj = b as Record<string, unknown>;
    const aKeys = Object.keys(aObj);
    const bKeys = Object.keys(bObj);
    if (aKeys.length !== bKeys.length) return false;
    for (const key of aKeys) {{
      if (!deepEqual(aObj[key], bObj[key])) return false;
    }}
    return true;
  }}
  return false;
}}

const testApi = {{
  test: (name: string, fn: TestFn) => addTest(name, fn),
  it: (name: string, fn: TestFn) => addTest(name, fn),
  describe: (name: string, fn: TestFn) => pushSuite(name, fn),
  beforeAll: (fn: TestFn) => addHook("beforeAll", fn),
  afterAll: (fn: TestFn) => addHook("afterAll", fn),
  beforeEach: (fn: TestFn) => addHook("beforeEach", fn),
  afterEach: (fn: TestFn) => addHook("afterEach", fn),
  expect: (value: unknown) => createExpect(value),
}};

(globalThis as any).dekaTest = {{
  spawn(args: string[], options: {{ command?: string; cwd?: string; env?: Record<string, string> }} = {{}}) {{
    const proc = (globalThis as any).process;
    if (!proc || typeof proc.spawn !== "function") {{
      throw new Error("process.spawn is not available");
    }}
    const command = options.command || (proc.env && proc.env.DEKA_BIN) || "deka";
    const child = proc.spawn(command, args, {{
      cwd: options.cwd,
      env: options.env,
    }});
    return child;
  }},
  waitForExit(child: any) {{
    return new Promise((resolve) => {{
      if (!child || typeof child.on !== "function") {{
        resolve(null);
        return;
      }}
      child.on("exit", (code: number) => resolve(code));
    }});
  }},
  async waitForPort(port: number, options: {{ timeoutMs?: number; intervalMs?: number }} = {{}}) {{
    const timeoutMs = options.timeoutMs ?? 10000;
    const intervalMs = options.intervalMs ?? 200;
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {{
      try {{
        const res = await fetch(`http://127.0.0.1:${{port}}/`);
        if (res) return true;
      }} catch {{}}
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    }}
    return false;
  }},
  sleep(ms: number) {{
    return new Promise((resolve) => setTimeout(resolve, ms));
  }},
}};

(globalThis as any).__dekaTest = testApi;

const files = {files_json};

serve({{
  async fetch() {{
    const summary = await runTests(files);
    const ok = summary.failed === 0;
    return new Response(JSON.stringify(summary), {{
      status: ok ? 200 : 500,
      headers: {{ "content-type": "application/json" }},
    }});
  }},
}});
"#,
        files_json = files_json,
        preloads_json = preloads_json,
        pattern_json = pattern_json
    )
}

fn create_temp_dir() -> Result<PathBuf, String> {
    let base = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("time error: {}", err))?
        .as_millis();
    let dir = base.join(format!("deka-test-{}-{}", std::process::id(), stamp));
    fs::create_dir_all(&dir).map_err(|err| format!("failed to create temp dir: {}", err))?;
    Ok(dir)
}

fn spawn_runtime(
    context: &Context,
    handler_path: &Path,
    port: &str,
) -> Result<std::process::Child, String> {
    let exe =
        std::env::current_exe().map_err(|err| format!("failed to resolve executable: {}", err))?;
    let exe_string = exe.to_string_lossy().to_string();
    let security_flags = security_flags_from_context(context);
    let mut cmd = Command::new(exe);
    cmd.arg("serve")
        .arg(handler_path)
        .args(&security_flags)
        .env("HANDLER_PATH", handler_path)
        .env("PORT", port)
        .env("DEKA_BIN", exe_string)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    cmd.spawn()
        .map_err(|err| format!("failed to start test runtime: {}", err))
}

fn security_flags_from_context(context: &Context) -> Vec<String> {
    const SECURITY_FLAGS: &[&str] = &[
        "--allow-read",
        "--allow-write",
        "--allow-net",
        "--allow-env",
        "--allow-run",
        "--allow-db",
        "--allow-dynamic",
        "--allow-wasm",
        "--allow-all",
        "--deny-read",
        "--deny-write",
        "--deny-net",
        "--deny-env",
        "--deny-run",
        "--deny-db",
        "--deny-dynamic",
        "--deny-wasm",
        "--no-prompt",
    ];
    SECURITY_FLAGS
        .iter()
        .filter(|name| context.args.flags.get(**name).copied().unwrap_or(false))
        .map(|name| (*name).to_string())
        .collect()
}

fn stop_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn fetch_with_timeout(port: &str, timeout: Duration) -> Option<Result<(u16, String), String>> {
    let port_num = match port.parse::<u16>() {
        Ok(port_num) => port_num,
        Err(err) => return Some(Err(format!("invalid port {}: {}", port, err))),
    };
    let addr: SocketAddr = format!("127.0.0.1:{}", port_num)
        .parse()
        .map_err(|err| format!("invalid address: {}", err))
        .ok()?;

    let stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| format!("failed to set read timeout: {}", err))
        .ok()?;
    let mut stream = stream;
    let request = format!("GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    if let Err(err) = stream.write_all(request.as_bytes()) {
        return Some(Err(format!("failed to write request: {}", err)));
    }
    let mut response = String::new();
    if let Err(err) = stream.read_to_string(&mut response) {
        return Some(Err(format!("failed to read response: {}", err)));
    }

    let mut headers = response.splitn(2, "\r\n\r\n");
    let head = headers.next().unwrap_or("");
    let body = headers.next().unwrap_or("").to_string();
    let status_line = head.lines().next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(500);
    Some(Ok((status, body)))
}
