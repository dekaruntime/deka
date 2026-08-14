use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use runtime_core::module_spec::module_spec_aliases;
use swc_bundler::{BundleKind, Bundler, Config, Hook, Load, ModuleData, ModuleType};
use swc_common::{
    FileName, GLOBALS, Globals, Mark, SourceMap, comments::SingleThreadedComments, sync::Lrc,
};
use swc_ecma_ast::{EsVersion, KeyValueProp, Module, Pass, Program};
use swc_ecma_codegen::{Emitter, text_writer::JsWriter};
use swc_ecma_loader::resolve::{Resolution, Resolve};
use swc_ecma_minifier::optimize;
use swc_ecma_minifier::option::{CompressOptions, MangleOptions, MinifyOptions};
use swc_ecma_parser::{EsSyntax, Parser, StringInput, Syntax, TsSyntax, lexer::Lexer};
use swc_ecma_transforms_base::helpers::Helpers;
use swc_ecma_transforms_base::resolver;
use swc_ecma_transforms_react::{Options as JsxOptions, Runtime as JsxRuntime, react};
use swc_ecma_transforms_typescript::strip;

pub use crate::cached::bundle_browser_assets_cached;
use crate::css_bundler::{self, CssAsset};

const REACT_SOURCE: &str = include_str!("../src-ts/vendor/react.esm.js");
const REACT_DOM_CLIENT_SOURCE: &str = include_str!("../src-ts/vendor/react-dom-client.esm.js");
const REACT_JSX_RUNTIME_SOURCE: &str = include_str!("../src-ts/vendor/react-jsx-runtime.esm.js");

pub struct JsBundle {
    pub code: String,
    pub css: Option<String>,
    pub assets: Vec<CssAsset>,
}

pub struct BundleOptions {
    pub project_root: PathBuf,
    pub minify: bool,
    pub iife: bool,
    /// Deprecated. The resolver only reads the project-local `php_modules/`.
    /// This field is retained for API compatibility and is ignored.
    /// Install stdlib packages via `deka install` — no filesystem fallback.
    pub stdlib_path: Option<PathBuf>,
}

pub type BuildOptions = BundleOptions;

/// Optimize already-emitted ESM without resolving or bundling imports.
///
/// Contract: source and output are ESM; relative specifiers are preserved.
/// This is the same safe SWC configuration used for `BundleOptions::minify`
/// and exists for the CLI's module-preserving `--treeshake` mode.
pub fn optimize_emitted_module(source: &str, path: &Path) -> Result<String, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        FileName::Real(path.to_path_buf()).into(),
        source.to_string(),
    );
    let syntax = Syntax::Es(EsSyntax {
        jsx: false,
        export_default_from: true,
        import_attributes: true,
        ..Default::default()
    });
    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let module = parser
        .parse_module()
        .map_err(|err| format!("failed to parse emitted JavaScript: {err:?}"))?;
    let globals = Globals::new();
    let module = GLOBALS.set(&globals, || minify_module(module, cm.clone()));
    emit_module(&module, cm)
}

fn minify_module(module: Module, cm: Lrc<SourceMap>) -> Module {
    let top_level_mark = Mark::new();
    let unresolved_mark = Mark::new();
    // These restrictions guard known SWC output bugs: conditionals/bools can
    // emit invalid assignment expressions, sequences can corrupt for-of heads,
    // inline can merge module-local bindings, and if_return can lose ternary
    // parentheses. Keep this shared configuration in sync for bundling and
    // module-preserving transpile optimization.
    let mut compress = CompressOptions::default();
    compress.conditionals = false;
    compress.bools = false;
    compress.sequences = 0;
    compress.inline = 0;
    compress.if_return = false;
    let minify_options = MinifyOptions {
        compress: Some(compress),
        mangle: None,
        ..Default::default()
    };
    match optimize(
        Program::Module(module),
        cm,
        None,
        None,
        &minify_options,
        &swc_ecma_minifier::option::ExtraOptions {
            unresolved_mark,
            top_level_mark,
            mangle_name_cache: Default::default(),
        },
    ) {
        Program::Module(module) => module,
        Program::Script(_) => unreachable!("module optimization returned a script"),
    }
}

fn emit_module(module: &Module, cm: Lrc<SourceMap>) -> Result<String, String> {
    let mut buf = Vec::new();
    let mut emitter = Emitter {
        cfg: swc_ecma_codegen::Config::default(),
        comments: None,
        cm: cm.clone(),
        wr: JsWriter::new(cm, "\n", &mut buf, None),
    };
    emitter
        .emit_module(module)
        .map_err(|err| format!("failed to emit optimized JavaScript: {err}"))?;
    String::from_utf8(buf).map_err(|err| format!("optimized JavaScript was not UTF-8: {err}"))
}

pub trait VirtualSource: Send + Sync {
    fn load_virtual(&self, path: &Path) -> Result<Option<String>, String>;
}

pub fn bundle_virtual_entry(
    entry_path: &Path,
    options: BundleOptions,
    provider: Arc<dyn VirtualSource>,
) -> Result<String, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let globals = Globals::new();
    let loader = VirtualLoader {
        cm: cm.clone(),
        css_collector: Arc::new(Mutex::new(CssCollector::default())),
        provider,
    };
    let resolver = DekaResolver::new(options.project_root, options.stdlib_path)?;

    let mut bundler = Bundler::new(
        &globals,
        cm.clone(),
        loader,
        resolver,
        Config {
            require: false,
            disable_inliner: false,
            disable_hygiene: false,
            disable_fixer: false,
            disable_dce: false,
            external_modules: Vec::new(),
            module: if options.iife {
                ModuleType::Iife
            } else {
                ModuleType::Es
            },
        },
        Box::new(NoopHook),
    );

    let mut entries = HashMap::new();
    entries.insert(
        "entry".to_string(),
        FileName::Real(entry_path.to_path_buf()),
    );

    let bundles = GLOBALS
        .set(&globals, || bundler.bundle(entries))
        .map_err(|err| format!("{err:?}"))?;
    let bundle = bundles
        .into_iter()
        .find(|bundle| matches!(bundle.kind, BundleKind::Named { .. }))
        .ok_or_else(|| "Failed to find bundled output".to_string())?;

    let module = if options.minify {
        GLOBALS.set(&globals, || minify_module(bundle.module, cm.clone()))
    } else {
        bundle.module
    };

    let mut buf = Vec::new();
    {
        let mut emitter = Emitter {
            cfg: swc_ecma_codegen::Config::default(),
            comments: None,
            cm: cm.clone(),
            wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
        };
        emitter
            .emit_module(&module)
            .map_err(|err| err.to_string())?;
    }

    String::from_utf8(buf).map_err(|err| err.to_string())
}

pub fn bundle_browser(entry: &str) -> Result<String, String> {
    let entry_path = resolve_entry(entry)?;
    let cm: Lrc<SourceMap> = Default::default();
    let globals = Globals::new();
    let root = std::env::current_dir().map_err(|err| err.to_string())?;
    let loader = FsLoader {
        cm: cm.clone(),
        css_collector: Arc::new(Mutex::new(CssCollector::default())),
        root: root.clone(),
    };
    let resolver = FsResolver::new()?;

    let mut bundler = Bundler::new(
        &globals,
        cm.clone(),
        loader,
        resolver,
        Config {
            require: false,
            disable_inliner: false,
            disable_hygiene: false,
            disable_fixer: false,
            disable_dce: false,
            external_modules: Vec::new(),
            module: ModuleType::Es,
        },
        Box::new(NoopHook),
    );

    let mut entries = HashMap::new();
    entries.insert("entry".to_string(), FileName::Real(entry_path));

    let bundles = GLOBALS
        .set(&globals, || bundler.bundle(entries))
        .map_err(|err| err.to_string())?;
    let bundle = bundles
        .into_iter()
        .find(|bundle| matches!(bundle.kind, BundleKind::Named { .. }))
        .ok_or_else(|| "Failed to find bundled output".to_string())?;

    let mut buf = Vec::new();
    {
        let mut emitter = Emitter {
            cfg: swc_ecma_codegen::Config::default(),
            comments: None,
            cm: cm.clone(),
            wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
        };
        emitter
            .emit_module(&bundle.module)
            .map_err(|err| err.to_string())?;
    }

    let code = String::from_utf8(buf).map_err(|err| err.to_string())?;
    Ok(code)
}

pub fn bundle_browser_assets(entry: &str) -> Result<JsBundle, String> {
    let entry_path = resolve_entry(entry)?;
    let cm: Lrc<SourceMap> = Default::default();
    let globals = Globals::new();
    let css_collector = Arc::new(Mutex::new(CssCollector::default()));
    let root = std::env::current_dir().map_err(|err| err.to_string())?;
    let loader = FsLoader {
        cm: cm.clone(),
        css_collector: Arc::clone(&css_collector),
        root: root.clone(),
    };
    let resolver = FsResolver::new()?;

    let mut bundler = Bundler::new(
        &globals,
        cm.clone(),
        loader,
        resolver,
        Config {
            require: false,
            disable_inliner: false,
            disable_hygiene: false,
            disable_fixer: false,
            disable_dce: false,
            external_modules: Vec::new(),
            module: ModuleType::Es,
        },
        Box::new(NoopHook),
    );

    let mut entries = HashMap::new();
    entries.insert("entry".to_string(), FileName::Real(entry_path));

    let bundles = GLOBALS
        .set(&globals, || bundler.bundle(entries))
        .map_err(|err| err.to_string())?;
    let bundle = bundles
        .into_iter()
        .find(|bundle| matches!(bundle.kind, BundleKind::Named { .. }))
        .ok_or_else(|| "Failed to find bundled output".to_string())?;

    let mut buf = Vec::new();
    {
        let mut emitter = Emitter {
            cfg: swc_ecma_codegen::Config::default(),
            comments: None,
            cm: cm.clone(),
            wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
        };
        emitter
            .emit_module(&bundle.module)
            .map_err(|err| err.to_string())?;
    }

    let code = String::from_utf8(buf).map_err(|err| err.to_string())?;
    let collector = css_collector
        .lock()
        .map_err(|_| "CSS collector lock failed".to_string())?;
    let css = collector
        .entries
        .iter()
        .map(|entry| entry.code.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let assets = collector
        .entries
        .iter()
        .flat_map(|entry| entry.assets.clone())
        .collect::<Vec<_>>();

    Ok(JsBundle {
        code,
        css: if css.is_empty() { None } else { Some(css) },
        assets,
    })
}

pub(crate) fn resolve_entry(entry: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(entry);
    if path.is_absolute() {
        return Ok(path);
    }
    let cwd = std::env::current_dir().map_err(|err| err.to_string())?;
    Ok(cwd.join(path))
}

#[derive(Default)]
struct CssCollector {
    seen: HashSet<PathBuf>,
    entries: Vec<CssEntry>,
}

struct CssEntry {
    code: String,
    assets: Vec<CssAsset>,
}

struct FsLoader {
    cm: Lrc<SourceMap>,
    css_collector: Arc<Mutex<CssCollector>>,
    /// Project root — second containment line after DekaResolver.
    /// Even if a future caller bypasses DekaResolver, the loader will not
    /// read files outside this directory.
    root: PathBuf,
}

impl Load for FsLoader {
    fn load(&self, file: &FileName) -> Result<ModuleData, anyhow::Error> {
        let (source, path) = match file {
            FileName::Real(path) => {
                // Defense-in-depth: verify the path stays within the project
                // root before touching the filesystem, even though DekaResolver
                // already enforces this for normal resolution paths.
                if guard_path_traversal(path, &self.root).is_none() {
                    anyhow::bail!(
                        "Path traversal blocked by FsLoader: {} is outside project root",
                        path.display()
                    );
                }
                if path.extension().and_then(|ext| ext.to_str()) == Some("css") {
                    let (source, module_path) = self.load_css_module(path)?;
                    (source, module_path)
                } else {
                    let source = std::fs::read_to_string(path).map_err(|err| {
                        anyhow::Error::msg(format!("Failed to read {}: {}", path.display(), err))
                    })?;
                    (source, Some(path.clone()))
                }
            }
            FileName::Custom(name) if name == "deka:react" => (REACT_SOURCE.to_string(), None),
            FileName::Custom(name) if name == "deka:react-dom-client" => {
                (REACT_DOM_CLIENT_SOURCE.to_string(), None)
            }
            FileName::Custom(name) if name == "deka:react-jsx-runtime" => {
                (REACT_JSX_RUNTIME_SOURCE.to_string(), None)
            }
            other => anyhow::bail!("Unsupported file name: {other:?}"),
        };

        let file_name = match file {
            FileName::Real(path) => FileName::Real(path.clone()),
            FileName::Custom(name) => FileName::Custom(name.clone()),
            other => other.clone(),
        };
        let fm = self.cm.new_source_file(file_name.into(), source);
        let syntax = match path {
            Some(ref path) => syntax_for_path(path),
            None => Syntax::Es(EsSyntax {
                jsx: false,
                export_default_from: true,
                import_attributes: true,
                ..Default::default()
            }),
        };
        let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
        let mut parser = Parser::new_from(lexer);
        let module = parser
            .parse_module()
            .map_err(|err| anyhow::Error::msg(format!("{:?}", err)))?;

        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();
        let mut program = Program::Module(module);
        let mut pass = resolver(unresolved_mark, top_level_mark, false);
        pass.process(&mut program);

        if path
            .as_ref()
            .map_or(false, |path| is_typescript(path.as_path()))
        {
            let mut pass = strip(unresolved_mark, top_level_mark);
            pass.process(&mut program);
        }

        if path.as_ref().map_or(false, |path| is_jsx(path.as_path())) {
            let mut options = JsxOptions::default();
            options.runtime = Some(JsxRuntime::Automatic);
            options.import_source = Some("react".into());
            let mut pass = react(
                self.cm.clone(),
                Some(SingleThreadedComments::default()),
                options,
                top_level_mark,
                unresolved_mark,
            );
            pass.process(&mut program);
        }

        let module = match program {
            Program::Module(module) => module,
            Program::Script(_) => anyhow::bail!("Unexpected script output when bundling module"),
        };

        Ok(ModuleData {
            fm,
            module,
            helpers: Helpers::new(false),
        })
    }
}

impl FsLoader {
    fn load_css_module(&self, path: &PathBuf) -> Result<(String, Option<PathBuf>), anyhow::Error> {
        let mut collector = self
            .css_collector
            .lock()
            .map_err(|_| anyhow::Error::msg("CSS collector lock failed"))?;
        let already_seen = collector.seen.contains(path);
        if !already_seen {
            collector.seen.insert(path.clone());
            let css_modules = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".module.css"))
                .unwrap_or(false);
            let bundle = css_bundler::bundle_css(&path.display().to_string(), css_modules, true)
                .map_err(|err| anyhow::Error::msg(err))?;
            collector.entries.push(CssEntry {
                code: bundle.code,
                assets: bundle.assets,
            });

            let module_source = if css_modules {
                let exports = bundle.exports.unwrap_or_default();
                let serialized = serde_json::to_string(&exports)
                    .map_err(|err| anyhow::Error::msg(err.to_string()))?;
                let mut lines = Vec::new();
                lines.push(format!("const styles = {};", serialized));
                lines.push("export default styles;".to_string());
                for key in exports.keys() {
                    append_named_exports(&mut lines, key)?;
                }
                lines.join("\n")
            } else {
                "export default {};".to_string()
            };

            return Ok((module_source, None));
        }

        let css_modules = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".module.css"))
            .unwrap_or(false);
        if css_modules {
            // Rebuild exports for repeated imports to keep module contents stable.
            let bundle = css_bundler::bundle_css(&path.display().to_string(), true, true)
                .map_err(|err| anyhow::Error::msg(err))?;
            let exports = bundle.exports.unwrap_or_default();
            let serialized = serde_json::to_string(&exports)
                .map_err(|err| anyhow::Error::msg(err.to_string()))?;
            let mut lines = Vec::new();
            lines.push(format!("const styles = {};", serialized));
            lines.push("export default styles;".to_string());
            for key in exports.keys() {
                append_named_exports(&mut lines, key)?;
            }
            return Ok((lines.join("\n"), None));
        }

        Ok(("export default {};".to_string(), None))
    }
}

struct VirtualLoader {
    cm: Lrc<SourceMap>,
    css_collector: Arc<Mutex<CssCollector>>,
    provider: Arc<dyn VirtualSource>,
}

impl Load for VirtualLoader {
    fn load(&self, file: &FileName) -> Result<ModuleData, anyhow::Error> {
        let (source, path) = match file {
            FileName::Real(path) => {
                if let Some(source) = self
                    .provider
                    .load_virtual(path)
                    .map_err(|err| anyhow::Error::msg(err))?
                {
                    (source, Some(path.clone()))
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("css") {
                    let (source, module_path) = self.load_css_module(path)?;
                    (source, module_path)
                } else {
                    let source = std::fs::read_to_string(path).map_err(|err| {
                        anyhow::Error::msg(format!("Failed to read {}: {}", path.display(), err))
                    })?;
                    (source, Some(path.clone()))
                }
            }
            FileName::Custom(name) if name == "deka:react" => (REACT_SOURCE.to_string(), None),
            FileName::Custom(name) if name == "deka:react-dom-client" => {
                (REACT_DOM_CLIENT_SOURCE.to_string(), None)
            }
            FileName::Custom(name) if name == "deka:react-jsx-runtime" => {
                (REACT_JSX_RUNTIME_SOURCE.to_string(), None)
            }
            other => anyhow::bail!("Unsupported file name: {other:?}"),
        };

        let file_name = match file {
            FileName::Real(path) => FileName::Real(path.clone()),
            FileName::Custom(name) => FileName::Custom(name.clone()),
            other => other.clone(),
        };
        let fm = self.cm.new_source_file(file_name.into(), source);
        let syntax = match path {
            Some(ref path) => syntax_for_path(path),
            None => Syntax::Es(EsSyntax {
                jsx: false,
                export_default_from: true,
                import_attributes: true,
                ..Default::default()
            }),
        };

        let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
        let mut parser = Parser::new_from(lexer);
        let module = parser
            .parse_module()
            .map_err(|err| anyhow::Error::msg(format!("{:?}", err)))?;

        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();
        let mut program = Program::Module(module);
        let mut pass = resolver(unresolved_mark, top_level_mark, false);
        pass.process(&mut program);

        if path
            .as_ref()
            .map_or(false, |path| is_typescript(path.as_path()))
        {
            let mut pass = strip(unresolved_mark, top_level_mark);
            pass.process(&mut program);
        }

        if path.as_ref().map_or(false, |path| is_jsx(path.as_path())) {
            let mut options = JsxOptions::default();
            options.runtime = Some(JsxRuntime::Automatic);
            options.import_source = Some("react".into());
            let mut pass = react(
                self.cm.clone(),
                Some(SingleThreadedComments::default()),
                options,
                top_level_mark,
                unresolved_mark,
            );
            pass.process(&mut program);
        }

        let module = match program {
            Program::Module(module) => module,
            Program::Script(_) => anyhow::bail!("Unexpected script output when bundling module"),
        };

        Ok(ModuleData {
            fm,
            module,
            helpers: Helpers::new(false),
        })
    }
}

impl VirtualLoader {
    fn load_css_module(&self, path: &PathBuf) -> Result<(String, Option<PathBuf>), anyhow::Error> {
        let mut collector = self
            .css_collector
            .lock()
            .map_err(|_| anyhow::Error::msg("CSS collector lock failed"))?;
        let already_seen = collector.seen.contains(path);
        if !already_seen {
            collector.seen.insert(path.clone());
            let css_modules = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".module.css"))
                .unwrap_or(false);
            let bundle = css_bundler::bundle_css(&path.display().to_string(), css_modules, true)
                .map_err(|err| anyhow::Error::msg(err))?;
            collector.entries.push(CssEntry {
                code: bundle.code,
                assets: bundle.assets,
            });

            let module_source = if css_modules {
                let exports = bundle.exports.unwrap_or_default();
                let serialized = serde_json::to_string(&exports)
                    .map_err(|err| anyhow::Error::msg(err.to_string()))?;
                let mut lines = Vec::new();
                lines.push(format!("const styles = {};", serialized));
                lines.push("export default styles;".to_string());
                for key in exports.keys() {
                    append_named_exports(&mut lines, key)?;
                }
                lines.join("\n")
            } else {
                "export default {};".to_string()
            };

            return Ok((module_source, None));
        }

        let css_modules = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".module.css"))
            .unwrap_or(false);
        if css_modules {
            let bundle = css_bundler::bundle_css(&path.display().to_string(), true, true)
                .map_err(|err| anyhow::Error::msg(err))?;
            let exports = bundle.exports.unwrap_or_default();
            let serialized = serde_json::to_string(&exports)
                .map_err(|err| anyhow::Error::msg(err.to_string()))?;
            let mut lines = Vec::new();
            lines.push(format!("const styles = {};", serialized));
            lines.push("export default styles;".to_string());
            for key in exports.keys() {
                append_named_exports(&mut lines, key)?;
            }
            return Ok((lines.join("\n"), None));
        }

        Ok(("export default {};".to_string(), None))
    }
}

struct FsResolver {
    root: PathBuf,
}

impl FsResolver {
    fn new() -> Result<Self, String> {
        let root = std::env::current_dir().map_err(|err| err.to_string())?;
        Ok(Self { root })
    }

    fn resolve_from_node_modules(&self, start_dir: &Path, specifier: &str) -> Option<PathBuf> {
        // Walk up the directory tree looking for node_modules
        let mut current = start_dir;
        loop {
            let node_modules = current.join("node_modules");
            if node_modules.is_dir() {
                // Return the path - let the candidate resolution handle extensions
                return Some(node_modules.join(specifier));
            }

            // Move up to parent directory
            current = current.parent()?;
        }
    }
}

impl Resolve for FsResolver {
    fn resolve(&self, base: &FileName, specifier: &str) -> Result<Resolution, anyhow::Error> {
        if specifier == "react" {
            return Ok(Resolution {
                filename: FileName::Custom("deka:react".to_string()),
                slug: None,
            });
        }
        if specifier == "react-dom/client" {
            return Ok(Resolution {
                filename: FileName::Custom("deka:react-dom-client".to_string()),
                slug: None,
            });
        }
        if specifier == "react/jsx-runtime" {
            return Ok(Resolution {
                filename: FileName::Custom("deka:react-jsx-runtime".to_string()),
                slug: None,
            });
        }
        if specifier == "deka/jsx-runtime" {
            return Ok(Resolution {
                filename: FileName::Custom("deka:react-jsx-runtime".to_string()),
                slug: None,
            });
        }

        let base_path = match base {
            FileName::Real(path) => path.clone(),
            _ => PathBuf::from("."),
        };
        let base_dir = base_path.parent().unwrap_or_else(|| Path::new("."));

        // Check if this is a node_modules import (bare specifier or scoped package)
        let is_node_module = !specifier.starts_with("./")
            && !specifier.starts_with("../")
            && !specifier.starts_with("/")
            && !specifier.starts_with("@/");

        let mut target = if specifier.starts_with("@/") {
            self.root.join(specifier.trim_start_matches("@/"))
        } else if specifier.starts_with('/') {
            PathBuf::from(specifier)
        } else if is_node_module {
            // Try to resolve from node_modules
            if let Some(resolved) = self.resolve_from_node_modules(base_dir, specifier) {
                resolved
            } else {
                // Fallback to treating as relative path
                base_dir.join(specifier)
            }
        } else {
            base_dir.join(specifier)
        };

        let mut candidates = Vec::new();
        if target.extension().is_none() {
            candidates.push(target.with_extension("ds"));
            candidates.push(target.with_extension("ts"));
            candidates.push(target.with_extension("tsx"));
            candidates.push(target.with_extension("jsx"));
            candidates.push(target.with_extension("js"));
            candidates.push(target.with_extension("mjs"));
            candidates.push(target.join("index.ds"));
            candidates.push(target.join("index.ts"));
            candidates.push(target.join("index.tsx"));
            candidates.push(target.join("index.jsx"));
            candidates.push(target.join("index.js"));
            candidates.push(target.join("index.mjs"));
        }
        candidates.push(target.clone());

        for candidate in candidates {
            if candidate.is_file() {
                target = candidate;
                return Ok(Resolution {
                    filename: FileName::Real(target),
                    slug: None,
                });
            }
        }

        anyhow::bail!("Unable to resolve {specifier} from {base:?}")
    }
}

struct NoopHook;

impl Hook for NoopHook {
    fn get_import_meta_props(
        &self,
        _span: swc_common::Span,
        _module_record: &swc_bundler::ModuleRecord,
    ) -> Result<Vec<KeyValueProp>, anyhow::Error> {
        Ok(Vec::new())
    }
}

fn syntax_for_path(path: &Path) -> Syntax {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "ds" => Syntax::Es(EsSyntax {
            jsx: false,
            export_default_from: true,
            import_attributes: true,
            ..Default::default()
        }),
        "ts" => Syntax::Typescript(TsSyntax {
            tsx: false,
            decorators: false,
            dts: false,
            no_early_errors: true,
            disallow_ambiguous_jsx_like: true,
        }),
        "tsx" => Syntax::Typescript(TsSyntax {
            tsx: true,
            decorators: false,
            dts: false,
            no_early_errors: true,
            disallow_ambiguous_jsx_like: true,
        }),
        "jsx" => Syntax::Es(EsSyntax {
            jsx: true,
            export_default_from: true,
            import_attributes: true,
            ..Default::default()
        }),
        _ => Syntax::Es(EsSyntax {
            jsx: false,
            export_default_from: true,
            import_attributes: true,
            ..Default::default()
        }),
    }
}

fn is_typescript(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("ds") => false,
        Some("ts") | Some("tsx") => true,
        _ => false,
    }
}

fn is_jsx(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("tsx") | Some("jsx") => true,
        _ => false,
    }
}

/// Verify that `resolved` stays within `root` after canonicalization.
/// Returns `None` if the path escapes the root (path traversal).
fn guard_path_traversal(resolved: &Path, root: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(resolved).ok()?;
    let canonical_root = std::fs::canonicalize(root).ok()?;
    if canonical.starts_with(&canonical_root) {
        Some(canonical)
    } else {
        None
    }
}

struct DekaResolver {
    root: PathBuf,
    php_modules: PathBuf,
}

impl DekaResolver {
    fn new(project_root: PathBuf, _stdlib_path: Option<PathBuf>) -> Result<Self, String> {
        let php_modules = project_root.join("php_modules");
        Ok(Self {
            root: project_root,
            php_modules,
        })
    }

    fn resolve_from_node_modules(&self, start_dir: &Path, specifier: &str) -> Option<PathBuf> {
        let mut current = start_dir;
        loop {
            let node_modules = current.join("node_modules");
            if node_modules.is_dir() {
                return Some(node_modules.join(specifier));
            }
            current = current.parent()?;
        }
    }

    fn resolve_php_module(&self, specifier: &str) -> Option<PathBuf> {
        for alias in module_spec_aliases(specifier) {
            // Only look in the project-local php_modules/. There is no system
            // stdlib fallback — packages must be installed via `deka install`.
            let base = if alias.starts_with("@user/") {
                self.php_modules
                    .join("@user")
                    .join(alias.trim_start_matches("@user/"))
            } else {
                self.php_modules.join(&alias)
            };
            if let Some(path) = resolve_with_candidates(&base) {
                // Guard against path traversal — resolved path must stay
                // within the php_modules directory.
                if guard_path_traversal(&path, &self.php_modules).is_some() {
                    return Some(path);
                }
            }
        }
        None
    }
}

impl Resolve for DekaResolver {
    fn resolve(&self, base: &FileName, specifier: &str) -> Result<Resolution, anyhow::Error> {
        if specifier == "react" {
            return Ok(Resolution {
                filename: FileName::Custom("deka:react".to_string()),
                slug: None,
            });
        }
        if specifier == "react-dom/client" {
            return Ok(Resolution {
                filename: FileName::Custom("deka:react-dom-client".to_string()),
                slug: None,
            });
        }
        if specifier == "react/jsx-runtime" {
            return Ok(Resolution {
                filename: FileName::Custom("deka:react-jsx-runtime".to_string()),
                slug: None,
            });
        }
        if specifier == "deka/jsx-runtime" {
            return Ok(Resolution {
                filename: FileName::Custom("deka:react-jsx-runtime".to_string()),
                slug: None,
            });
        }

        let base_path = match base {
            FileName::Real(path) => path.clone(),
            _ => self.root.clone(),
        };
        let base_dir = base_path.parent().unwrap_or_else(|| self.root.as_path());

        if specifier.starts_with("@/") {
            let target = self.root.join(specifier.trim_start_matches("@/"));
            if let Some(candidate) = resolve_with_candidates(&target) {
                // Guard: resolved @/ path must stay within the project root
                if guard_path_traversal(&candidate, &self.root).is_some() {
                    return Ok(Resolution {
                        filename: FileName::Real(candidate),
                        slug: None,
                    });
                }
            }
        }

        let is_prefixed_module = specifier.starts_with("component/")
            || specifier.starts_with("encoding/")
            || specifier.starts_with("deka/")
            || specifier.starts_with("db/")
            || specifier.starts_with("core/");
        if is_prefixed_module {
            // Project-local php_modules/ only — no system stdlib fallback.
            if let Some(candidate) = resolve_with_candidates(&self.php_modules.join(specifier)) {
                // Guard: resolved path must stay within php_modules/
                if guard_path_traversal(&candidate, &self.php_modules).is_some() {
                    return Ok(Resolution {
                        filename: FileName::Real(candidate),
                        slug: None,
                    });
                }
            }
            // Try the @deka/ scoped layout: encoding/json -> @deka/encoding/json
            let scoped_specifier = format!("@deka/{}", specifier);
            if let Some(candidate) =
                resolve_with_candidates(&self.php_modules.join(&scoped_specifier))
            {
                if guard_path_traversal(&candidate, &self.php_modules).is_some() {
                    return Ok(Resolution {
                        filename: FileName::Real(candidate),
                        slug: None,
                    });
                }
            }
        }

        if let Some(candidate) = self.resolve_php_module(specifier) {
            return Ok(Resolution {
                filename: FileName::Real(candidate),
                slug: None,
            });
        }

        let is_node_module = !specifier.starts_with("./")
            && !specifier.starts_with("../")
            && !specifier.starts_with('/')
            && !specifier.starts_with("@/");

        let mut target = if specifier.starts_with('/') {
            PathBuf::from(specifier)
        } else if is_node_module {
            if let Some(resolved) = self.resolve_from_node_modules(base_dir, specifier) {
                resolved
            } else {
                base_dir.join(specifier)
            }
        } else {
            base_dir.join(specifier)
        };

        if let Some(candidate) = resolve_with_candidates(&target) {
            target = candidate;
        }

        if target.is_file() {
            // Guard: resolved relative path must stay within the project root.
            // This closes the path-traversal gap for `../` imports that resolve
            // to a file that actually exists outside the project root.
            if guard_path_traversal(&target, &self.root).is_none() {
                anyhow::bail!(
                    "Path traversal detected: {specifier} from {base:?} resolves outside project root"
                );
            }
            return Ok(Resolution {
                filename: FileName::Real(target),
                slug: None,
            });
        }

        anyhow::bail!("Unable to resolve {specifier} from {base:?}")
    }
}

fn resolve_with_candidates(target: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if target.extension().is_none() {
        candidates.push(target.with_extension("ds"));
        candidates.push(target.with_extension("ts"));
        candidates.push(target.with_extension("tsx"));
        candidates.push(target.with_extension("jsx"));
        candidates.push(target.with_extension("js"));
        candidates.push(target.with_extension("mjs"));
        candidates.push(target.join("index.ds"));
        candidates.push(target.join("index.ts"));
        candidates.push(target.join("index.tsx"));
        candidates.push(target.join("index.jsx"));
        candidates.push(target.join("index.js"));
        candidates.push(target.join("index.mjs"));
    }
    candidates.push(target.to_path_buf());

    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn append_named_exports(lines: &mut Vec<String>, key: &str) -> Result<(), anyhow::Error> {
    let key_json = serde_json::to_string(key).map_err(|err| anyhow::Error::msg(err.to_string()))?;
    let mut names = Vec::new();
    if is_valid_identifier(key) {
        names.push(key.to_string());
    }
    let camel = to_camel_case(key);
    if !camel.is_empty() && camel != key && is_valid_identifier(&camel) {
        names.push(camel);
    }
    for name in names {
        lines.push(format!("export const {} = styles[{}];", name, key_json));
    }
    Ok(())
}

fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(ch) => ch,
        None => return false,
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    for ch in chars {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$') {
            return false;
        }
    }
    true
}

fn to_camel_case(input: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for ch in input.chars() {
        if ch == '-' || ch == '_' || ch == ' ' {
            upper = true;
            continue;
        }
        if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests;
