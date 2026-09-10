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
pub(crate) fn entry_wrapper_source(entry_specifier: &str) -> String {
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
fn host_bindings_preamble(kinds: &[String]) -> String {
    let grants_json = serde_json::to_string(kinds).unwrap_or_else(|_| "[]".to_string());
    format!(
        "const __dekaHostBindings = globalThis[Symbol.for('deka.host.internal')];\n\
         const __dekaModuleGrants = Object.freeze({grants_json});\n\
         const __deka_host = __dekaHostBindings && ((k, a, args) => __dekaHostBindings.host(k, a, args, __dekaModuleGrants));\n\
         const __deka_to_result = __dekaHostBindings && __dekaHostBindings.toResult;\n"
    )
}

pub(crate) fn prepend_host_bindings(code: ModuleSourceCode, kinds: &[String]) -> ModuleSourceCode {
    match code {
        ModuleSourceCode::String(source) => {
            let preamble = host_bindings_preamble(kinds);
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

    #[test]
    fn wrapper_accepts_exported_dekascript_app() {
        let source = entry_wrapper_source("file:///main.ds");
        assert!(source.contains("__dekaMain.App"));
        assert!(source.contains("ui/router"));
        assert!(source.contains("file:///main.ds"));
    }
}
