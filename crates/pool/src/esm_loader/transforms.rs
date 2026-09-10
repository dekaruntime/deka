//! Source-code transforms applied to modules as they are loaded: the entry
//! wrapper template, the per-module RFD 27 host-bindings preamble, and the
//! entry footer that exports a bare `app` binding to `globalThis`.
//!
//! These functions only rewrite `ModuleSourceCode::String` payloads; other
//! variants pass through untouched.

use deno_core::ModuleSourceCode;

/// Entry wrapper module source. Imports the `ui/*` toolchain namespaces onto
/// `globalThis.deka.ui`, dynamically imports the entry, and assigns the best
/// candidate export (`default`, `app`, `App`, `handler`, or the namespace) to
/// `globalThis.app`, adapting function/object candidates where a runtime
/// adapter is present.
/// Generate the loader-owned entry wrapper. Build emission also reads this
/// source to seed the artifact's UI dependency graph: the wrapper is executed
/// for every entry, so its imports are artifact dependencies just as much as
/// imports written by an app module are.
pub fn entry_wrapper_source(entry_specifier: &str) -> String {
    let template = "import * as __jsx from \"ui/jsx\";\n\
import * as __server from \"ui/server\";\n\
import * as __reactive from \"ui/reactive\";\n\
import * as __suspense from \"ui/suspense\";\n\
import * as __router from \"ui/router\";\n\
globalThis.deka = globalThis.deka || {};\n\
globalThis.deka.ui = Object.freeze({\n\
  ...(globalThis.deka.ui || {}),\n\
  ...__jsx,\n\
  ...__server,\n\
  ...__reactive,\n\
  ...__suspense,\n\
  ...__router,\n\
});\n\
const __dekaMain = await import(\"__ENTRY__\");\n\
globalThis.__dekaStaticRender =\n\
  typeof __dekaMain.StaticRender === \"function\"\n\
    ? __dekaMain.StaticRender\n\
    : undefined;\n\
globalThis.__dekaBuild =\n\
  typeof __dekaMain.default === \"function\"\n\
    ? __dekaMain.default\n\
    : undefined;\n\
const __candidate = typeof __dekaMain.default !== \"undefined\"\n\
  ? __dekaMain.default\n\
  : typeof __dekaMain.app !== \"undefined\"\n\
  ? __dekaMain.app\n\
  : typeof __dekaMain.App !== \"undefined\"\n\
  ? __dekaMain.App\n\
  : typeof __dekaMain.handler !== \"undefined\"\n\
  ? __dekaMain.handler\n\
  : __dekaMain;\n\
if (typeof globalThis.app === \"undefined\" && typeof __candidate !== \"undefined\") {\n\
  if (typeof __candidate === \"function\" && typeof globalThis.__dekaNodeExpressAdapter === \"function\" && (typeof __candidate.handle === \"function\" || typeof __candidate.listen === \"function\")) {\n\
    globalThis.app = globalThis.__dekaNodeExpressAdapter(__candidate);\n\
  } else if (__candidate && typeof __candidate === \"object\" && typeof __candidate.fetch === \"function\") {\n\
    globalThis.app = __candidate;\n\
  } else if (__candidate && typeof __candidate === \"object\" && !__candidate.__dekaServer && typeof __candidate.routes === \"object\" && globalThis.__deka && typeof globalThis.__deka.serve === \"function\") {\n\
    globalThis.app = globalThis.__deka.serve(__candidate);\n\
  } else {\n\
    globalThis.app = __candidate;\n\
  }\n\
}\n";
    template.replace("__ENTRY__", entry_specifier)
}

/// Generate the per-module host-bindings preamble. Every DekaScript module
/// gets a `__deka_host` closure that carries only its package's granted kinds
/// — this is the "every bridge site" RFD 27 gate. Local names `__deka_host`
/// and `__deka_to_result` are part of the dsc emit contract (and the deka_ui
/// fallback references them), so they must not be renamed.
///
/// RFD 21 (deka#754): modules classified as official stdlib additionally get
/// a module-scoped `deka` binding resolving to the closed catalog object on
/// the realm-private internal surface. Non-stdlib modules get no binding, so
/// their `deka` references keep resolving through the global scope (today's
/// `deka.ui` behavior) and the compiled-JS gate refuses catalog kinds.
fn host_bindings_preamble(kinds: &[String], catalog: bool) -> String {
    let grants_json = serde_json::to_string(kinds).unwrap_or_else(|_| "[]".to_string());
    let mut preamble = format!(
        "const __dekaHostBindings = globalThis[Symbol.for('deka.host.internal')];\n\
         const __dekaModuleGrants = Object.freeze({grants_json});\n\
         const __deka_host = __dekaHostBindings && ((k, a, args) => __dekaHostBindings.host(k, a, args, __dekaModuleGrants));\n\
         const __deka_to_result = __dekaHostBindings && __dekaHostBindings.toResult;\n"
    );
    if catalog {
        preamble.push_str(
            "const deka = __dekaHostBindings && __dekaHostBindings.moduleDeka\n\
               ? __dekaHostBindings.moduleDeka(true)\n\
               : globalThis.deka;\n",
        );
    }
    preamble
}

pub(crate) fn prepend_host_bindings(
    code: ModuleSourceCode,
    kinds: &[String],
    catalog: bool,
) -> ModuleSourceCode {
    match code {
        ModuleSourceCode::String(source) => {
            let preamble = host_bindings_preamble(kinds, catalog);
            let mut text = String::with_capacity(preamble.len() + source.len());
            text.push_str(&preamble);
            text.push_str(&source);
            ModuleSourceCode::String(text.into())
        }
        other => other,
    }
}

pub(crate) fn append_entry_footer(code: ModuleSourceCode) -> ModuleSourceCode {
    const FOOTER: &str = "\nif (typeof globalThis.app === \"undefined\" && typeof app !== \"undefined\") {\n\
  const __candidate = app;\n\
  if (typeof __candidate === \"function\" && typeof globalThis.__dekaNodeExpressAdapter === \"function\" && (typeof __candidate.handle === \"function\" || typeof __candidate.listen === \"function\")) {\n\
    globalThis.app = globalThis.__dekaNodeExpressAdapter(__candidate);\n\
  } else if (__candidate && typeof __candidate === \"object\" && typeof __candidate.fetch === \"function\") {\n\
    globalThis.app = __candidate;\n\
  } else if (__candidate && typeof __candidate === \"object\" && !__candidate.__dekaServer && typeof __candidate.routes === \"object\" && globalThis.__deka && typeof globalThis.__deka.serve === \"function\") {\n\
    globalThis.app = globalThis.__deka.serve(__candidate);\n\
  } else {\n\
    globalThis.app = __candidate;\n\
  }\n\
}\n";

    match code {
        ModuleSourceCode::String(source) => {
            let mut text = source.to_owned();
            text.push_str(FOOTER);
            ModuleSourceCode::String(text.into())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::entry_wrapper_source;
    use super::prepend_host_bindings;
    use deno_core::ModuleSourceCode;

    #[test]
    fn wrapper_accepts_exported_dekascript_app() {
        let source = entry_wrapper_source("file:///main.ds");
        assert!(source.contains("__dekaMain.App"));
        assert!(source.contains("ui/router"));
        assert!(source.contains("file:///main.ds"));
    }

    #[test]
    fn catalog_binding_is_injected_only_for_stdlib_modules() {
        let make = || ModuleSourceCode::String("export const n = 1".to_string().into());
        let stdlib = prepend_host_bindings(make(), &["crypto".to_string()], true);
        let user = prepend_host_bindings(make(), &[], false);
        let ModuleSourceCode::String(stdlib) = stdlib else {
            panic!("string in, string out")
        };
        let ModuleSourceCode::String(user) = user else {
            panic!("string in, string out")
        };
        // Stdlib modules get the module-scoped `deka` catalog binding, routed
        // through the realm-private internal surface.
        assert!(stdlib.contains("const deka = __dekaHostBindings && __dekaHostBindings.moduleDeka"));
        assert!(stdlib.contains("moduleDeka(true)"));
        // User modules get no `deka` binding at all: their references keep
        // resolving through the global scope (today's deka.ui behavior) and
        // the loader's compiled-JS gate refuses catalog kinds.
        assert!(!user.contains("moduleDeka"));
        assert!(!user.contains("const deka ="));
    }
}
