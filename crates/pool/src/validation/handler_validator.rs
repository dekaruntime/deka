//! Handler Validation with swc AST Parser
//!
//! Provides detailed, helpful error messages when handlers:
//! - Try to use Node.js APIs (this is not Node.js!)
//! - Try to use Cloudflare Workers APIs (this is Deka!)
//! - Import unknown deka/* modules
//! - Don't export a default handler
use swc_common::{FileName, SourceMap, Span, Spanned, sync::Lrc};
use swc_ecma_ast::*;
use swc_ecma_parser::{Parser, StringInput, Syntax, TsSyntax, error::SyntaxError, lexer::Lexer};
use swc_ecma_visit::{Visit, VisitWith};

use super::error_formatter::format_validation_error;

/// Available deka modules
const DEKA_MODULES: &[(&str, &[&str], &str)] = &[
    (
        "deka",
        &["Mesh", "IsolatePool", "Isolate", "serve"],
        "import { Mesh, IsolatePool, Isolate, serve } from 'deka'\nconst mesh = new Mesh()\nconst pool = new IsolatePool(mesh)\nconst server = serve({ routes: {} })",
    ),
    (
        "deka/router",
        &[
            "Router",
            "Context",
            "cors",
            "logger",
            "basicAuth",
            "bearerAuth",
            "rateLimit",
            "prettyJSON",
        ],
        "import { Router } from 'deka/router'\nconst app = new Router()\napp.get('/', (c) => c.json({ ok: true }))\nexport default app",
    ),
    (
        "deka/postgres",
        &["query", "execute"],
        "import { query } from 'deka/postgres'\nconst rows = await query('SELECT * FROM users')",
    ),
    (
        "deka/docker",
        &["createContainer", "listContainers"],
        "import { createContainer } from 'deka/docker'\nawait createContainer({ image: 'nginx' })",
    ),
    (
        "deka/t4",
        &["t4", "write", "T4Client", "T4File"],
        "import { t4 } from 'deka/t4'\nconst file = t4.file('data.json')\nconst data = await file.json()",
    ),
    (
        "deka/sqlite",
        &["Database", "Statement"],
        "import { Database } from 'deka/sqlite'\nconst db = new Database('mydb.sqlite')\nconst users = db.query('SELECT * FROM users').all()",
    ),
];

/// Find deka module definition
fn find_deka_module(name: &str) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    DEKA_MODULES
        .iter()
        .find(|(module_name, _, _)| *module_name == name)
        .map(|(name, exports, example)| (*name, *exports, *example))
}

/// Format available modules list
fn format_module_list() -> String {
    DEKA_MODULES
        .iter()
        .map(|(name, exports, _)| format!("  • {} - exports: {}", name, exports.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Validate handler source code and provide helpful error messages
pub fn validate_handler(source_code: &str, file_path: &str) -> Result<(), String> {
    // Create source map for parsing
    let cm: Lrc<SourceMap> = Default::default();

    // Configure parser (enable JSX for .tsx/.jsx)
    let lower = file_path.to_ascii_lowercase();
    let is_jsx = lower.ends_with(".tsx") || lower.ends_with(".jsx");
    let syntax = Syntax::Typescript(TsSyntax {
        tsx: is_jsx,
        decorators: false,
        dts: false,
        no_early_errors: true,
        disallow_ambiguous_jsx_like: true,
    });

    // Create input directly from source string
    let fm = cm.new_source_file(
        FileName::Custom(file_path.to_string()).into(),
        source_code.to_string(),
    );

    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);

    let mut parser = Parser::new_from(lexer);

    // Parse the module
    let module = match parser.parse_module() {
        Ok(m) => m,
        Err(e) => {
            return Err(format_parse_error(source_code, file_path, &cm, e));
        }
    };

    // Validate imports
    validate_imports(&module, source_code, file_path, &cm)?;

    // Validate exports (must have default export)
    validate_exports(&module, source_code, file_path)?;

    Ok(())
}

#[derive(Default, Debug, Clone)]
pub struct ServeOptions {
    pub port: Option<u16>,
    pub unix: Option<String>,
    pub tcp: Option<String>,
    pub udp: Option<String>,
    pub dns: Option<String>,
    pub ws: Option<u16>,
    pub redis: Option<String>,
    pub workers: Option<PoolWorkers>,
    pub isolates_per_worker: Option<usize>,
}

pub fn extract_serve_options(source_code: &str, file_path: &str) -> ServeOptions {
    let cm: Lrc<SourceMap> = Default::default();
    let lower = file_path.to_ascii_lowercase();
    let syntax = Syntax::Typescript(TsSyntax {
        tsx: lower.ends_with(".tsx") || lower.ends_with(".jsx"),
        decorators: false,
        dts: false,
        no_early_errors: true,
        disallow_ambiguous_jsx_like: true,
    });

    let fm = cm.new_source_file(
        FileName::Custom(file_path.to_string()).into(),
        source_code.to_string(),
    );

    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let module = match parser.parse_module() {
        Ok(module) => module,
        Err(_) => return ServeOptions::default(),
    };

    let mut extractor = ServeOptionsExtractor::default();
    extractor.visit_module(&module);
    extractor.options
}

#[derive(Default, Debug, Clone)]
pub struct PoolOptions {
    pub workers: Option<PoolWorkers>,
    pub isolates_per_worker: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum PoolWorkers {
    Fixed(usize),
    Max,
}

pub fn extract_pool_options(source_code: &str, file_path: &str) -> PoolOptions {
    let cm: Lrc<SourceMap> = Default::default();
    let lower = file_path.to_ascii_lowercase();
    let syntax = Syntax::Typescript(TsSyntax {
        tsx: lower.ends_with(".tsx") || lower.ends_with(".jsx"),
        decorators: false,
        dts: false,
        no_early_errors: true,
        disallow_ambiguous_jsx_like: true,
    });

    let fm = cm.new_source_file(
        FileName::Custom(file_path.to_string()).into(),
        source_code.to_string(),
    );

    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let module = match parser.parse_module() {
        Ok(module) => module,
        Err(_) => return PoolOptions::default(),
    };

    let mut extractor = PoolOptionsExtractor::default();
    extractor.visit_module(&module);
    extractor.options
}

#[derive(Default)]
struct PoolOptionsExtractor {
    options: PoolOptions,
}

impl PoolOptionsExtractor {
    fn apply_from_object(&mut self, obj: &ObjectLit) {
        for prop in &obj.props {
            let prop = match prop {
                PropOrSpread::Prop(prop) => prop,
                PropOrSpread::Spread(_) => continue,
            };

            let (key, value) = match &**prop {
                Prop::KeyValue(kv) => (prop_name_to_string(&kv.key), &*kv.value),
                _ => continue,
            };

            if key == "workers" {
                if self.options.workers.is_none() {
                    if let Some(value) = extract_usize_literal(value) {
                        self.options.workers = Some(PoolWorkers::Fixed(value));
                    } else if let Some(value) = extract_string_literal(value) {
                        if value.eq_ignore_ascii_case("max") || value.eq_ignore_ascii_case("auto") {
                            self.options.workers = Some(PoolWorkers::Max);
                        }
                    }
                }
            } else if key == "isolatesPerWorker" || key == "isolates_per_worker" {
                if self.options.isolates_per_worker.is_none() {
                    if let Some(value) = extract_usize_literal(value) {
                        self.options.isolates_per_worker = Some(value);
                    }
                }
            }
        }
    }
}

impl Visit for PoolOptionsExtractor {
    fn visit_new_expr(&mut self, n: &NewExpr) {
        if is_isolate_pool_new(&n.callee) {
            if let Some(args) = &n.args {
                if args.len() >= 2 {
                    if let Some(second) = args.get(1) {
                        if let Expr::Object(obj) = &*second.expr {
                            self.apply_from_object(obj);
                        }
                    }
                } else if let Some(first) = args.get(0) {
                    if let Expr::Object(obj) = &*first.expr {
                        self.apply_from_object(obj);
                    }
                }
            }
        }
        n.visit_children_with(self);
    }
}

fn is_isolate_pool_new(callee: &Expr) -> bool {
    match callee {
        Expr::Ident(ident) => ident.sym == *"IsolatePool",
        Expr::Member(member) => match &*member.obj {
            Expr::Ident(obj_ident) => obj_ident.sym == *"Deka",
            _ => false,
        },
        _ => false,
    }
}

#[derive(Default)]
struct ServeOptionsExtractor {
    options: ServeOptions,
}

impl ServeOptionsExtractor {
    fn apply_from_object(&mut self, obj: &ObjectLit) {
        for prop in &obj.props {
            let prop = match prop {
                PropOrSpread::Prop(prop) => prop,
                PropOrSpread::Spread(_) => continue,
            };

            let (key, value) = match &**prop {
                Prop::KeyValue(kv) => (prop_name_to_string(&kv.key), &*kv.value),
                _ => continue,
            };

            if key == "port" {
                if self.options.port.is_none() {
                    if let Some(port) = extract_u16_literal(value) {
                        self.options.port = Some(port);
                    }
                }
            } else if key == "unix" {
                if self.options.unix.is_none() {
                    if let Some(unix) = extract_string_literal(value) {
                        self.options.unix = Some(unix);
                    }
                }
            } else if key == "tcp" {
                if self.options.tcp.is_none() {
                    if let Some(addr) = extract_string_literal(value) {
                        self.options.tcp = Some(addr);
                    }
                }
            } else if key == "udp" {
                if self.options.udp.is_none() {
                    if let Some(addr) = extract_string_literal(value) {
                        self.options.udp = Some(addr);
                    }
                }
            } else if key == "dns" {
                if self.options.dns.is_none() {
                    if let Some(addr) = extract_string_literal(value) {
                        self.options.dns = Some(addr);
                    }
                }
            } else if key == "ws" {
                if self.options.ws.is_none() {
                    if let Some(port) = extract_u16_literal(value) {
                        self.options.ws = Some(port);
                    }
                }
            } else if key == "redis" {
                if self.options.redis.is_none() {
                    if let Some(addr) = extract_string_literal(value) {
                        self.options.redis = Some(addr);
                    }
                }
            } else if key == "workers" {
                if self.options.workers.is_none() {
                    if let Some(value) = extract_usize_literal(value) {
                        self.options.workers = Some(PoolWorkers::Fixed(value));
                    } else if let Some(value) = extract_string_literal(value) {
                        if value.eq_ignore_ascii_case("max") || value.eq_ignore_ascii_case("auto") {
                            self.options.workers = Some(PoolWorkers::Max);
                        }
                    }
                }
            } else if key == "isolatesPerWorker" || key == "isolates_per_worker" {
                if self.options.isolates_per_worker.is_none() {
                    if let Some(value) = extract_usize_literal(value) {
                        self.options.isolates_per_worker = Some(value);
                    }
                }
            }
        }
    }
}

impl Visit for ServeOptionsExtractor {
    fn visit_call_expr(&mut self, n: &CallExpr) {
        if is_serve_call(&n.callee) {
            if let Some(first) = n.args.get(0) {
                if let Expr::Object(obj) = &*first.expr {
                    self.apply_from_object(obj);
                }
            }
        }

        n.visit_children_with(self);
    }

    fn visit_export_default_expr(&mut self, n: &ExportDefaultExpr) {
        if let Expr::Object(obj) = &*n.expr {
            self.apply_from_object(obj);
        }
        n.visit_children_with(self);
    }
}

fn is_serve_call(callee: &Callee) -> bool {
    match callee {
        Callee::Expr(expr) => match &**expr {
            Expr::Ident(ident) => ident.sym == *"serve",
            Expr::Member(member) => match &*member.obj {
                Expr::Ident(obj_ident) => obj_ident.sym == *"Deka" || obj_ident.sym == *"Bun",
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

fn prop_name_to_string(name: &PropName) -> String {
    match name {
        PropName::Ident(ident) => ident.sym.to_string(),
        PropName::Str(str) => str.value.to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

fn extract_string_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(Lit::Str(str)) => Some(str.value.to_string_lossy().into_owned()),
        _ => None,
    }
}

fn extract_usize_literal(expr: &Expr) -> Option<usize> {
    match expr {
        Expr::Lit(Lit::Num(num)) => {
            let value = num.value;
            if value.is_finite() && value >= 0.0 && (value - value.floor()).abs() < f64::EPSILON {
                Some(value as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn extract_u16_literal(expr: &Expr) -> Option<u16> {
    match expr {
        Expr::Lit(Lit::Num(num)) => {
            if num.value >= 0.0 && num.value <= u16::MAX as f64 {
                Some(num.value as u16)
            } else {
                None
            }
        }
        Expr::Lit(Lit::Str(str)) => str.value.to_string_lossy().parse::<u16>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod serve_option_tests {
    use super::extract_serve_options;

    #[test]
    fn extracts_serve_call_options() {
        let source = r#"
import { serve } from "deka"

const server = serve({ port: 9001, unix: "/tmp/deka.sock" })
"#;
        let options = extract_serve_options(source, "handler.ts");
        assert_eq!(options.port, Some(9001));
        assert_eq!(options.unix.as_deref(), Some("/tmp/deka.sock"));
    }

    #[test]
    fn extracts_default_export_options() {
        let source = r#"
export default {
  unix: "/tmp/deka-default.sock",
  fetch() {
    return new Response("ok")
  }
}
"#;
        let options = extract_serve_options(source, "handler.ts");
        assert_eq!(options.unix.as_deref(), Some("/tmp/deka-default.sock"));
    }
}

/// Validate imports - only deka/* modules and relative imports allowed
fn validate_imports(
    module: &Module,
    source_code: &str,
    file_path: &str,
    cm: &SourceMap,
) -> Result<(), String> {
    for item in &module.body {
        if let ModuleItem::ModuleDecl(ModuleDecl::Import(import_decl)) = item {
            let import_str = format!("{:?}", &import_decl.src.value)
                .trim_matches('"')
                .to_string();
            let span = import_decl.src.span;

            // Allow deka module namespace
            if import_str == "deka" || import_str.starts_with("deka/") {
                // Validate it's a known deka module
                if find_deka_module(&import_str).is_none() {
                    let (line_num, col_num, underline_length) = extract_span_info(cm, span);
                    return Err(format_validation_error(
                        source_code,
                        file_path,
                        "Invalid Import",
                        line_num,
                        col_num,
                        &format!("Unknown deka module: {}", import_str),
                        &format!(
                            "Available deka modules:\n\
                                {}\n\
                                \n\
                                Check the documentation: https://docs.deka.gg",
                            format_module_list()
                        ),
                        underline_length,
                    ));
                }

                // Validate imported names match what the module exports
                validate_deka_imports(
                    &import_str,
                    &import_decl.specifiers,
                    source_code,
                    file_path,
                    cm,
                )?;
                continue;
            }

            // Allow relative imports
            if import_str.starts_with("./") || import_str.starts_with("../") {
                continue;
            }

            // Block Node.js built-ins
            let node_builtins = [
                "fs",
                "path",
                "os",
                "crypto",
                "http",
                "https",
                "net",
                "child_process",
                "cluster",
                "dgram",
                "dns",
                "domain",
                "events",
                "stream",
                "util",
                "v8",
                "vm",
                "zlib",
                "process",
                "buffer",
                "assert",
                "querystring",
                "url",
                "string_decoder",
                "timers",
                "tls",
                "tty",
                "readline",
                "repl",
                "constants",
                "module",
                "perf_hooks",
                "async_hooks",
                "worker_threads",
                "inspector",
                "trace_events",
            ];

            if node_builtins.contains(&import_str.as_str()) || import_str.starts_with("node:") {
                let (line_num, col_num, underline_length) = extract_span_info(cm, span);
                return Err(format_validation_error(
                    source_code,
                    file_path,
                    "Invalid Import",
                    line_num,
                    col_num,
                    "Node.js modules are not available in deka-runtime",
                    &format!(
                        "This is not Node.js! Deka provides a limited, secure runtime.\n\
                        \n\
                        You tried to import: '{}'\n\
                        \n\
                        Instead, use Deka's built-in modules:\n\
                        • deka/router - HTTP routing\n\
                        • deka/postgres - Database access\n\
                        • deka/docker - Container management\n\
                        • deka/t4 - File storage\n\
                        \n\
                        Learn more: https://docs.deka.gg/guides/node-to-deka",
                        import_str
                    ),
                    underline_length,
                ));
            }

            // Block Cloudflare Workers APIs
            if import_str.starts_with("cloudflare:") {
                let (line_num, col_num, underline_length) = extract_span_info(cm, span);
                return Err(format_validation_error(
                    source_code,
                    file_path,
                    "Invalid Import",
                    line_num,
                    col_num,
                    "Cloudflare Workers APIs are not available in Deka",
                    &format!(
                        "This is not Cloudflare Workers! This is Deka.\n\
                        \n\
                        You tried to import: '{}'\n\
                        \n\
                        Deka provides similar but different APIs:\n\
                        • Use 'deka/t4' for storage (like KV/R2)\n\
                        • Use 'deka/router' for HTTP routing\n\
                        • Use 'deka/postgres' for databases\n\
                        \n\
                        Learn more: https://docs.deka.gg/guides/workers-to-deka",
                        import_str
                    ),
                    underline_length,
                ));
            }

            // Block Deno-style HTTP imports
            if import_str.starts_with("https://") || import_str.starts_with("http://") {
                let (line_num, col_num, underline_length) = extract_span_info(cm, span);
                return Err(format_validation_error(
                    source_code,
                    file_path,
                    "Invalid Import",
                    line_num,
                    col_num,
                    "HTTP imports are not supported in Deka",
                    &format!(
                        "Deka does not support Deno-style HTTP imports.\n\
                        \n\
                        You tried to import: '{}'\n\
                        \n\
                        Use Deka's built-in modules instead:\n\
                        • deka/router, deka/postgres, deka/docker, etc.\n\
                        \n\
                        All dependencies must be bundled with your handler.",
                        import_str
                    ),
                    underline_length,
                ));
            }

            // Allow bare specifiers for node_modules resolution.
        }
    }

    Ok(())
}

/// Validate that imported names match what a deka module actually exports
fn validate_deka_imports(
    module_name: &str,
    specifiers: &[ImportSpecifier],
    source_code: &str,
    file_path: &str,
    cm: &SourceMap,
) -> Result<(), String> {
    let module_def = find_deka_module(module_name);
    if module_def.is_none() {
        return Ok(()); // Unknown module, caught elsewhere
    }
    let (_, exports, example) = module_def.unwrap();

    for specifier in specifiers {
        match specifier {
            ImportSpecifier::Named(named) => {
                let imported_name = format!("{:?}", named.local.sym)
                    .trim_matches('"')
                    .to_string();
                let span = named.span;

                if !exports.contains(&imported_name.as_str()) {
                    let (line_num, col_num, underline_length) = extract_span_info(cm, span);
                    return Err(format_validation_error(
                        source_code,
                        file_path,
                        "Invalid Export",
                        line_num,
                        col_num,
                        &format!("'{}' is not exported by {}", imported_name, module_name),
                        &format!(
                            "{} only exports: {}\n\
                            \n\
                            Example usage:\n\
                            {}",
                            module_name,
                            exports.join(", "),
                            example
                        ),
                        underline_length,
                    ));
                }
            }
            ImportSpecifier::Default(_) => {
                return Err("Default imports are not supported for deka modules. Use named imports like: import { Router } from 'deka/router'".to_string());
            }
            ImportSpecifier::Namespace(_) => {
                return Err("Namespace imports are not supported for deka modules. Use named imports like: import { Router } from 'deka/router'".to_string());
            }
        }
    }

    Ok(())
}

/// Validate exports - must have default export
fn validate_exports(module: &Module, source_code: &str, file_path: &str) -> Result<(), String> {
    let mut has_default_export = false;

    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(_)) => {
                has_default_export = true;
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(_)) => {
                has_default_export = true;
            }
            _ => {}
        }
    }

    if !has_default_export {
        let has_serve = source_code.contains("serve(") || source_code.contains("serve (");
        if has_serve {
            return Ok(());
        }

        // Find last line for error position
        let last_line = source_code.lines().count();
        return Err(format_validation_error(
            source_code,
            file_path,
            "Missing Export",
            last_line,
            1,
            "Handler must export a default value",
            "Deka handlers must export a default value (usually a Router instance).\n\
            \n\
            Example:\n\
            import { Router } from 'deka/router'\n\
            const app = new Router()\n\
            app.get('/', (c) => c.json({ ok: true }))\n\
            export default app  // ← Add this!\n\
            \n\
            The default export should be:\n\
            • A Router instance (most common)\n\
            • A function that handles requests\n\
            • Any object with a .fetch(request) method",
            1,
        ));
    }

    Ok(())
}

/// Extract line number, column number, and underline length from a span
fn extract_span_info(cm: &SourceMap, span: Span) -> (usize, usize, usize) {
    let loc_start = cm.lookup_char_pos(span.lo);
    let loc_end = cm.lookup_char_pos(span.hi);
    let line_num = loc_start.line;
    let col_num = loc_start.col.0 + 1;

    // Calculate span width
    let underline_length = if loc_start.line == loc_end.line {
        (loc_end.col.0.saturating_sub(loc_start.col.0)).max(1)
    } else {
        10 // Multi-line span
    };

    (line_num, col_num, underline_length)
}

fn format_parse_error(
    source: &str,
    path: &str,
    cm: &SourceMap,
    err: swc_ecma_parser::error::Error,
) -> String {
    let span = err.span();
    let (line, col, underline_length) = extract_span_info(cm, span);

    let (message, hint) = match err.kind() {
        SyntaxError::Eof => (
            "Unexpected end of file".to_string(),
            "Check for an unclosed block, string, or parenthesis.".to_string(),
        ),
        SyntaxError::UnterminatedStrLit => (
            "Unterminated string literal".to_string(),
            "Add the missing closing quote.".to_string(),
        ),
        SyntaxError::UnterminatedTpl => (
            "Unterminated template literal".to_string(),
            "Add the missing closing backtick or ${} bracket.".to_string(),
        ),
        SyntaxError::UnterminatedRegExp => (
            "Unterminated regular expression".to_string(),
            "Add the missing closing /.".to_string(),
        ),
        SyntaxError::InvalidStrEscape => (
            "Invalid string escape".to_string(),
            "Check backslash escapes like \\n, \\t, or \\\".".to_string(),
        ),
        SyntaxError::InvalidUnicodeEscape => (
            "Invalid unicode escape".to_string(),
            "Use \\uXXXX or \\u{...} for unicode escapes.".to_string(),
        ),
        SyntaxError::ExpectedUnicodeEscape => (
            "Expected unicode escape".to_string(),
            "After \\u, provide four hex digits or \\u{...}.".to_string(),
        ),
        SyntaxError::TopLevelAwaitInScript => (
            "Top-level await is not allowed here".to_string(),
            "Wrap in an async function or use a module context.".to_string(),
        ),
        SyntaxError::Unexpected { got, expected } => (
            format!("Unexpected token {}, expected {}", got, expected),
            "Check for missing punctuation or a stray character.".to_string(),
        ),
        _ => (
            format!("{:?}", err.kind()),
            "Check the syntax near the highlighted location.".to_string(),
        ),
    };

    format_validation_error(
        source,
        path,
        "Parse Error",
        line,
        col,
        &message,
        &hint,
        underline_length,
    )
}
