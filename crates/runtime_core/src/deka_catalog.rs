//! RFD 21 runtime bridge — the closed `deka.*` catalog of JavaScript helpers.
//!
//! Stdlib packages reach platform operations Deka owns through exactly two
//! doors, both calling JavaScript **we shipped** (never inline platform JS):
//!
//! ```ds
//! safe { deka.bytes.len(b) }      // helper cannot throw; yields its declared type T
//! unsafe { deka.json.parse(s) }   // helper may throw; yields Result<T, E>
//! ```
//!
//! This module is the single authoritative catalog for that surface, the way
//! [`crate::host_bridge`] is for `bridge kind.action` (RFD 27). The pinned
//! `dsc` compiler performs no catalog validation — it types the ambient
//! `deka` global as `Infer` and emits plain calls verbatim — so the loader
//! scans stdlib `.ds` sources against [`DEKA_CATALOG`] before compiling and
//! refuses unknown helpers, wrong arity, or user-package use with a source
//! diagnostic. [`CATALOG_HELPERS_JS`] is the shipped implementation; it is
//! installed realm-private by the isolate bootstrap and reached only through
//! the per-module preamble binding, never through an ambient global.
//!
//! Contract:
//!
//! - **Closed catalog.** A `deka.*` call outside this table is a compile
//!   error. New helpers are a reviewed addition here — zero stubs: every
//!   entry below is genuinely implemented in [`CATALOG_HELPERS_JS`].
//! - **Classification is the return type** (RFD 21 rule 3): a helper that
//!   can throw is [`Safety::Unsafe`] and is only reachable under
//!   `unsafe { }`, where dsc's try/catch converts the throw into a
//!   `Result.Err` value the caller must handle. A [`Safety::Safe`] helper
//!   never throws for arguments of its declared types and never fabricates a
//!   fallback: failure is returned as a value (`Option.None`, `false`, …)
//!   per RFD 21 rule 4.
//! - **RFD 15 over RFD 21 on bytes.** RFD 21's table types `to_string` /
//!   `to_string_lossy` as lossy `safe` helpers (`TextDecoder` `fatal:false`)
//!   and lists `set` and `subarray`. This catalog instead follows RFD 15's
//!   stricter shape, which deka#756 (bytes) will inherit: bytes-to-string is
//!   a **strict, fallible decode** (`deka.bytes.to_string` is `Unsafe` and
//!   throws on invalid UTF-8, surfacing `Err`; there is deliberately no
//!   lossy variant — silent replacement characters are the defect RFD 15
//!   exists to remove), `bytes` is **immutable** (no `set`), and `slice`
//!   **copies** (no `subarray` view can alias a buffer and escape as
//!   immutable bytes).
//! - **No environment influence** (deka#801): the catalog is a `const`
//!   table; nothing here reads the process environment or `deka.json`. The
//!   loader may consult a project's `deka.json` only to decide whether a
//!   *package* is an official `@deka/*` stdlib package — never to change
//!   what the catalog contains.

/// A catalog value type — the DS-level type of an argument or success value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValType {
    /// `f64` / integer number.
    Number,
    /// UTF-8 string.
    String,
    /// Boolean.
    Bool,
    /// Immutable byte payload (`Uint8Array` representation, RFD 15).
    Bytes,
    /// `void` / unit.
    Unit,
    /// `Array<number>` (e.g. a byte list before `from_array`).
    NumList,
    /// Free-form JSON value (`unknown` on the DS surface).
    Any,
}

impl ValType {
    pub fn name(self) -> &'static str {
        match self {
            ValType::Number => "number",
            ValType::String => "string",
            ValType::Bool => "boolean",
            ValType::Bytes => "bytes",
            ValType::Unit => "void",
            ValType::NumList => "Array<number>",
            ValType::Any => "unknown",
        }
    }
}

/// The success shape of a helper, which fixes how the DS surface types it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnShape {
    /// `safe`: yields `T` directly.
    Value(ValType),
    /// `safe`: total, but partial — failure is `Option.None`, never a throw.
    OptionValue(ValType),
    /// `unsafe`: yields `Result<T, string>`; the `Err` payload is the thrown
    /// value's message, per dsc's bare `unsafe { }` contract (dsc#103).
    ResultValue(ValType),
}

/// Whether a helper can throw for arguments of its declared types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Safety {
    /// Cannot throw; reachable under `safe { }` (and, redundantly, `unsafe`).
    Safe,
    /// May throw; reachable only under `unsafe { }`, which maps the throw to
    /// a `Result.Err` value.
    Unsafe,
}

/// One positional argument of a catalog helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogArg {
    pub name: &'static str,
    pub ty: ValType,
    /// Trailing optional arguments (`end?`) may be omitted at the call site.
    pub optional: bool,
}

const fn arg(name: &'static str, ty: ValType) -> CatalogArg {
    CatalogArg { name, ty, optional: false }
}

const fn opt_arg(name: &'static str, ty: ValType) -> CatalogArg {
    CatalogArg { name, ty, optional: true }
}

/// A single catalog helper: `deka.<kind>.<name>(args...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogMethod {
    pub name: &'static str,
    pub safety: Safety,
    pub args: &'static [CatalogArg],
    pub ret: ReturnShape,
    /// One-line contract note for diagnostics and docs.
    pub doc: &'static str,
}

/// A catalog kind and its helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogKind {
    pub name: &'static str,
    pub methods: &'static [CatalogMethod],
}

const BYTES_METHODS: &[CatalogMethod] = &[
    CatalogMethod {
        name: "len",
        safety: Safety::Safe,
        args: &[arg("b", ValType::Bytes)],
        ret: ReturnShape::Value(ValType::Number),
        doc: "byte length (byteLength)",
    },
    CatalogMethod {
        name: "get",
        safety: Safety::Safe,
        args: &[arg("b", ValType::Bytes), arg("index", ValType::Number)],
        // RFD 21: "out of range is a value, not a throw".
        ret: ReturnShape::OptionValue(ValType::Number),
        doc: "indexed byte; out of range is None, not a throw",
    },
    CatalogMethod {
        name: "slice",
        safety: Safety::Safe,
        args: &[
            arg("b", ValType::Bytes),
            arg("start", ValType::Number),
            opt_arg("end", ValType::Number),
        ],
        // RFD 15: copies. There is no `subarray` — a view could alias a
        // mutable buffer and escape as immutable bytes.
        ret: ReturnShape::Value(ValType::Bytes),
        doc: "copy of the byte range (never a view)",
    },
    CatalogMethod {
        name: "concat",
        safety: Safety::Safe,
        args: &[arg("a", ValType::Bytes), arg("b", ValType::Bytes)],
        ret: ReturnShape::Value(ValType::Bytes),
        doc: "fresh buffer, alloc + copy",
    },
    CatalogMethod {
        name: "from_array",
        safety: Safety::Safe,
        args: &[arg("items", ValType::NumList)],
        ret: ReturnShape::Value(ValType::Bytes),
        doc: "Uint8Array.from",
    },
    CatalogMethod {
        name: "from_string",
        safety: Safety::Safe,
        args: &[arg("s", ValType::String)],
        ret: ReturnShape::Value(ValType::Bytes),
        doc: "TextEncoder.encode — total",
    },
    CatalogMethod {
        name: "to_hex",
        safety: Safety::Safe,
        args: &[arg("b", ValType::Bytes)],
        ret: ReturnShape::Value(ValType::String),
        doc: "lowercase hex; we own the loop",
    },
    CatalogMethod {
        name: "from_hex",
        safety: Safety::Safe,
        args: &[arg("s", ValType::String)],
        // RFD 21 rule 4: invalid input maps to Option.None — a value the
        // caller must handle, never a fabricated buffer.
        ret: ReturnShape::OptionValue(ValType::Bytes),
        doc: "hex decode; invalid input is None",
    },
    CatalogMethod {
        name: "to_base64",
        safety: Safety::Safe,
        args: &[arg("b", ValType::Bytes)],
        ret: ReturnShape::Value(ValType::String),
        doc: "standard base64; we own the loop",
    },
    CatalogMethod {
        name: "from_base64",
        safety: Safety::Safe,
        args: &[arg("s", ValType::String)],
        ret: ReturnShape::OptionValue(ValType::Bytes),
        doc: "base64 decode; invalid input is None",
    },
    CatalogMethod {
        name: "to_string",
        // RFD 15 over RFD 21: strict UTF-8 decode (TextDecoder fatal:true)
        // throws on invalid input, so the throw surfaces as Result.Err under
        // `unsafe`. There is intentionally no lossy variant.
        safety: Safety::Unsafe,
        args: &[arg("b", ValType::Bytes)],
        ret: ReturnShape::ResultValue(ValType::String),
        doc: "strict UTF-8 decode; invalid input is Err, never lossy",
    },
];

const JSON_METHODS: &[CatalogMethod] = &[
    CatalogMethod {
        name: "parse",
        safety: Safety::Unsafe,
        args: &[arg("s", ValType::String)],
        ret: ReturnShape::ResultValue(ValType::Any),
        doc: "JSON.parse — throws on malformed input",
    },
    CatalogMethod {
        name: "stringify",
        safety: Safety::Unsafe,
        args: &[arg("v", ValType::Any)],
        ret: ReturnShape::ResultValue(ValType::String),
        doc: "JSON.stringify — throws on cycles / bigint",
    },
    CatalogMethod {
        name: "validate",
        safety: Safety::Safe,
        args: &[arg("s", ValType::String)],
        // RFD 21 rule 4: the helper maps the throw to a boolean, so this is
        // total and classified safe.
        ret: ReturnShape::Value(ValType::Bool),
        doc: "true iff the input parses; never throws",
    },
];

const IO_METHODS: &[CatalogMethod] = &[
    CatalogMethod {
        name: "echo",
        safety: Safety::Safe,
        args: &[arg("message", ValType::String)],
        ret: ReturnShape::Value(ValType::Unit),
        doc: "one line of program output; console.log rebound to stdout",
    },
];

const TIME_METHODS: &[CatalogMethod] = &[
    CatalogMethod {
        name: "now",
        safety: Safety::Safe,
        args: &[],
        ret: ReturnShape::Value(ValType::Number),
        doc: "milliseconds since the Unix epoch; Date.now does not throw",
    },
];

/// The single authoritative `deka.*` catalog (RFD 21). Closed: a call outside
/// this table is a source diagnostic. Every entry is genuinely implemented in
/// [`CATALOG_HELPERS_JS`] — zero stubs. RFD 21's `deka.math` / `deka.string`
/// / `deka.cookies` families are deliberately absent until their owning
/// migration lands them (RFD 21 order steps 4–5), not stubbed.
pub const DEKA_CATALOG: &[CatalogKind] = &[
    CatalogKind { name: "bytes", methods: BYTES_METHODS },
    CatalogKind { name: "json", methods: JSON_METHODS },
    CatalogKind { name: "io", methods: IO_METHODS },
    CatalogKind { name: "time", methods: TIME_METHODS },
];

/// Look up a kind by name.
pub fn find_kind(name: &str) -> Option<&'static CatalogKind> {
    DEKA_CATALOG.iter().find(|kind| kind.name == name)
}

/// Look up a method by kind and method name.
pub fn find_method(kind: &str, method: &str) -> Option<&'static CatalogMethod> {
    find_kind(kind)?.methods.iter().find(|candidate| candidate.name == method)
}

/// Whether `name` is a catalog kind (used by the loader's compiled-JS gate).
pub fn is_catalog_kind(name: &str) -> bool {
    find_kind(name).is_some()
}

/// Arity bounds for a method: `(required, total)`.
pub fn arity_bounds(method: &CatalogMethod) -> (usize, usize) {
    let total = method.args.len();
    let required = method.args.iter().take_while(|a| !a.optional).count();
    (required, total)
}

/// Validate a call's argument count, returning a caller-facing message.
pub fn check_arity(method: &CatalogMethod, argc: usize) -> Result<(), String> {
    let (required, total) = arity_bounds(method);
    if argc < required || argc > total {
        let expected = if required == total {
            format!("{total}")
        } else {
            format!("{required}..{total}")
        };
        return Err(format!(
            "expected {expected} argument(s), found {argc}"
        ));
    }
    Ok(())
}

/// JavaScript implementation of every catalog entry — the "our JS" RFD 21's
/// two doors call. The isolate bootstrap evaluates this expression once per
/// worker and installs the frozen result on the realm-private internal
/// surface; modules reach it only through the per-module preamble `deka`
/// binding, so nothing here is published on `globalThis` and nothing mutates
/// a prototype. It is an expression (not declarations) so evaluating it can
/// never leak a binding into the realm.
///
/// Helpers take only the argument types their catalog entry declares. `safe`
/// helpers are total over those types; `Option` results are built from the
/// realm prelude constructors so the value is a real DS `Option` on the
/// surface. `unsafe` helpers throw on failure and are compiled under
/// dsc's try/catch, which converts the throw into `Result.Err`.
pub const CATALOG_HELPERS_JS: &str = r#"
(function () {
  "use strict";
  const some = (value) => Option.Some(value);
  const none = /* @__PURE__ */ Option.None();
  const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

  const bytes = {
    len(b) { return b.byteLength; },
    get(b, index) {
      const i = index | 0;
      return i >= 0 && i < b.byteLength ? some(b[i]) : none;
    },
    slice(b, start, end) {
      // Uint8Array#slice copies; the result never aliases `b`.
      return b.slice(start, end === undefined ? b.byteLength : end);
    },
    concat(a, b) {
      const out = new Uint8Array(a.byteLength + b.byteLength);
      out.set(a, 0);
      out.set(b, a.byteLength);
      return out;
    },
    from_array(items) { return Uint8Array.from(items); },
    from_string(s) { return new TextEncoder().encode(s); },
    to_hex(b) {
      let out = "";
      for (let i = 0; i < b.byteLength; i++) out += b[i].toString(16).padStart(2, "0");
      return out;
    },
    from_hex(s) {
      if (s.length % 2 !== 0) return none;
      const out = new Uint8Array(s.length / 2);
      for (let i = 0; i < out.length; i++) {
        const byte = parseInt(s.slice(i * 2, i * 2 + 2), 16);
        if (Number.isNaN(byte)) return none;
        out[i] = byte;
      }
      return some(out);
    },
    to_base64(b) {
      let out = "";
      for (let i = 0; i < b.byteLength; i += 3) {
        const n = (b[i] << 16) | ((i + 1 < b.byteLength ? b[i + 1] : 0) << 8) | (i + 2 < b.byteLength ? b[i + 2] : 0);
        out += BASE64[(n >> 18) & 63] + BASE64[(n >> 12) & 63]
          + (i + 1 < b.byteLength ? BASE64[(n >> 6) & 63] : "=")
          + (i + 2 < b.byteLength ? BASE64[n & 63] : "=");
      }
      return out;
    },
    from_base64(s) {
      if (s.length % 4 !== 0) return none;
      const clean = s.replace(/=+$/, "");
      let length = (clean.length * 3) / 4;
      if (clean.length % 4 === 2) length -= 2;
      else if (clean.length % 4 === 3) length -= 1;
      else if (clean.length % 4 !== 0) return none;
      const out = new Uint8Array(length);
      let o = 0;
      for (let i = 0; i < clean.length; i += 4) {
        const chunk = [0, 1, 2, 3].map((k) => {
          const c = i + k < clean.length ? clean[i + k] : "A";
          const v = BASE64.indexOf(c);
          return v < 0 ? -1 : v;
        });
        if (chunk.some((v) => v < 0)) return none;
        const n = (chunk[0] << 18) | (chunk[1] << 12) | (chunk[2] << 6) | chunk[3];
        if (o < out.length) out[o++] = (n >> 16) & 255;
        if (o < out.length) out[o++] = (n >> 8) & 255;
        if (o < out.length) out[o++] = n & 255;
      }
      return some(out);
    },
    to_string(b) {
      // Strict UTF-8 (RFD 15): throws on invalid input so the failure is an
      // Err value under `unsafe` — never a lossy substitution.
      return new TextDecoder("utf-8", { fatal: true }).decode(b);
    },
  };

  const json = {
    parse(s) { return JSON.parse(s); },
    stringify(v) { return JSON.stringify(v); },
    validate(s) {
      try { JSON.parse(s); return true; } catch (_) { return false; }
    },
  };

  const io = {
    echo(message) { console.log(message); },
  };

  const time = {
    now() { return Date.now(); },
  };

  return Object.freeze({
    bytes: Object.freeze(bytes),
    json: Object.freeze(json),
    io: Object.freeze(io),
    time: Object.freeze(time),
  });
})()
"#;

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_closed_and_wellformed() {
        assert!(!DEKA_CATALOG.is_empty());
        for kind in DEKA_CATALOG {
            assert!(!kind.name.is_empty());
            assert!(!kind.methods.is_empty(), "kind '{}' has no methods", kind.name);
            for method in kind.methods {
                assert!(!method.name.is_empty(), "{}. method with empty name", kind.name);
                assert!(!method.doc.is_empty(), "{}.{} has no doc", kind.name, method.name);
                let mut optional_seen = false;
                for a in method.args {
                    assert!(!a.name.is_empty());
                    if a.optional {
                        optional_seen = true;
                    } else {
                        assert!(!optional_seen, "{}.{}: required arg after optional", kind.name, method.name);
                    }
                }
                // Safety and return shape agree (RFD 21 rule 3): a Result
                // shape means the helper throws -> Unsafe.
                match (method.safety, method.ret) {
                    (Safety::Unsafe, ReturnShape::ResultValue(_)) => {}
                    (Safety::Safe, ReturnShape::Value(_) | ReturnShape::OptionValue(_)) => {}
                    _ => panic!(
                        "{}.{}: safety {:?} disagrees with return shape {:?}",
                        kind.name, method.name, method.safety, method.ret
                    ),
                }
            }
            // No duplicate method names within a kind.
            for (i, a) in kind.methods.iter().enumerate() {
                for b in &kind.methods[..i] {
                    assert_ne!(a.name, b.name, "kind '{}' duplicates '{}'", kind.name, a.name);
                }
            }
        }
    }

    #[test]
    fn every_entry_is_findable_and_unknown_names_are_not() {
        for kind in DEKA_CATALOG {
            assert_eq!(find_kind(kind.name).map(|k| k.name), Some(kind.name));
            assert!(is_catalog_kind(kind.name));
            for method in kind.methods {
                let found = find_method(kind.name, method.name).expect("find_method");
                assert_eq!(found.name, method.name);
            }
        }
        assert!(find_kind("bogus").is_none());
        assert!(find_method("bytes", "bogus").is_none());
        assert!(find_method("bogus", "len").is_none());
        assert!(!is_catalog_kind("bogus"));
    }

    #[test]
    fn arity_counts_top_level_arguments_only() {
        let get = find_method("bytes", "get").unwrap();
        assert_eq!(arity_bounds(get), (2, 2));
        assert!(check_arity(get, 2).is_ok());
        assert!(check_arity(get, 1).is_err());
        assert!(check_arity(get, 3).is_err());

        let slice = find_method("bytes", "slice").unwrap();
        assert_eq!(arity_bounds(slice), (2, 3));
        assert!(check_arity(slice, 2).is_ok());
        assert!(check_arity(slice, 3).is_ok());
        assert!(check_arity(slice, 1).is_err());
        assert!(check_arity(slice, 4).is_err());

        let now = find_method("time", "now").unwrap();
        assert_eq!(arity_bounds(now), (0, 0));
        assert!(check_arity(now, 0).is_ok());
        assert!(check_arity(now, 1).is_err());
    }

    /// RFD 15's stricter bytes shape is the contract deka#756 inherits. Pin it:
    /// strict fallible decode, immutability, no aliasing views, no lossy path.
    #[test]
    fn bytes_family_follows_rfd15_not_rfd21_lossy_shape() {
        // `to_string` is Unsafe (strict UTF-8, Err on failure) — RFD 21's
        // table says safe/lossy; RFD 15 wins (deka#756 inherits this).
        let to_string = find_method("bytes", "to_string").expect("to_string present");
        assert_eq!(to_string.safety, Safety::Unsafe);
        assert_eq!(to_string.ret, ReturnShape::ResultValue(ValType::String));

        // No lossy variant: silent replacement characters are the defect
        // RFD 15 exists to remove.
        assert!(find_method("bytes", "to_string_lossy").is_none());

        // Immutable bytes: no mutating `set`.
        assert!(find_method("bytes", "set").is_none());

        // No view-producing `subarray`; `slice` copies (RFD 15).
        assert!(find_method("bytes", "subarray").is_none());
        let slice = find_method("bytes", "slice").unwrap();
        assert_eq!(slice.ret, ReturnShape::Value(ValType::Bytes));

        // Total conversions stay safe; fallible decoders return Option values.
        assert_eq!(find_method("bytes", "from_string").unwrap().safety, Safety::Safe);
        assert_eq!(
            find_method("bytes", "from_hex").unwrap().ret,
            ReturnShape::OptionValue(ValType::Bytes)
        );
        assert_eq!(
            find_method("bytes", "get").unwrap().ret,
            ReturnShape::OptionValue(ValType::Number)
        );
    }

    #[test]
    fn classification_is_by_return_type() {
        assert_eq!(find_method("json", "parse").unwrap().safety, Safety::Unsafe);
        assert_eq!(find_method("json", "stringify").unwrap().safety, Safety::Unsafe);
        assert_eq!(
            find_method("json", "validate").unwrap().ret,
            ReturnShape::Value(ValType::Bool)
        );
        assert_eq!(find_method("io", "echo").unwrap().safety, Safety::Safe);
        assert_eq!(find_method("time", "now").unwrap().safety, Safety::Safe);
    }

    /// Emitted-JS proof (issue acceptance): the shipped helper source must not
    /// publish on globalThis and must not mutate prototypes.
    #[test]
    fn helper_js_never_touches_global_this_or_prototypes() {
        assert!(!CATALOG_HELPERS_JS.contains("globalThis"), "catalog helpers must not touch globalThis");
        assert!(!CATALOG_HELPERS_JS.contains(".prototype."), "catalog helpers must not mutate prototypes");
        assert!(!CATALOG_HELPERS_JS.contains("Object.assign"), "frozen literal surface only");
    }

    /// Every catalog entry has a shipped JS implementation with the same name
    /// — the "zero stubs" rule: nothing is catalogued that does not exist.
    #[test]
    fn every_catalog_entry_is_implemented_in_helper_js() {
        for kind in DEKA_CATALOG {
            for method in kind.methods {
                let needle = format!("{}( ", method.name);
                let needle2 = format!("{}(", method.name);
                assert!(
                    CATALOG_HELPERS_JS.contains(&needle) || CATALOG_HELPERS_JS.contains(&needle2),
                    "deka.{}.{} is catalogued but not implemented in CATALOG_HELPERS_JS",
                    kind.name,
                    method.name
                );
            }
        }
    }

    /// deka#801: the catalog must be uninfluenceable by the process
    /// environment. Poison every DEKA_* override and prove lookups are
    /// unchanged.
    #[test]
    fn catalog_is_environment_independent() {
        for (key, value) in [
            ("DEKA_CATALOG", "bytes.len=bogus"),
            ("DEKA_HOST_GRANTS", "[]"),
            ("DEKA_SECURITY_POLICY", "{}"),
            ("DEKA_DSC", "/nonexistent"),
        ] {
            // SAFETY: test-only env mutation; serialized by the test harness.
            unsafe { std::env::set_var(key, value) };
        }
        let looked_up = (
            find_method("bytes", "len").map(|m| m.safety),
            find_method("bytes", "bogus"),
            DEKA_CATALOG.len(),
        );
        for key in ["DEKA_CATALOG", "DEKA_HOST_GRANTS", "DEKA_SECURITY_POLICY", "DEKA_DSC"] {
            // SAFETY: test-only env restore; serialized by the test harness.
            unsafe { std::env::remove_var(key) };
        }
        assert_eq!(looked_up.0, Some(Safety::Safe));
        assert_eq!(looked_up.1, None);
        assert!(looked_up.2 >= 4);
    }

    /// Node-driven behavior proof when node is available (skipped otherwise):
    /// safe helpers never throw and return their declared shape, including the
    /// failure paths; unsafe helpers throw for the failure path.
    #[test]
    fn helper_js_behavior_via_node_when_available() {
        let Ok(node) = std::process::Command::new("node")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
        else {
            eprintln!("node not available; skipping helper behavior proof");
            return;
        };
        if !node {
            eprintln!("node not available; skipping helper behavior proof");
            return;
        }
        let script = format!(
            r#"
const Option = {{
  Some: (value) => ({{ __enum: "Option", __case: "Some", name: "Some", value }}),
  None: () => ({{ __enum: "Option", __case: "None", name: "None" }}),
}};
const c = {CATALOG_HELPERS_JS};
const assert = require("node:assert/strict");
// safe: total, declared types.
assert.equal(c.bytes.len(c.bytes.from_string("abc")), 3);
assert.equal(c.bytes.get(c.bytes.from_string("ab"), 5).__case, "None");
assert.equal(c.bytes.get(c.bytes.from_string("ab"), 1).value, 98);
const sl = c.bytes.slice(c.bytes.from_string("abcd"), 1, 3);
assert.deepEqual([...sl], [98, 99]);
const cat = c.bytes.concat(c.bytes.from_string("ab"), c.bytes.from_string("cd"));
assert.deepEqual([...cat], [97, 98, 99, 100]);
assert.deepEqual([...c.bytes.from_array([1, 2, 255])], [1, 2, 255]);
assert.equal(c.bytes.to_hex(c.bytes.from_string("ab")), "6162");
assert.deepEqual([...c.bytes.from_hex("6162").value], [97, 98]);
// failure path: a value the caller must handle, never fabricated bytes.
assert.equal(c.bytes.from_hex("zz").__case, "None");
assert.equal(c.bytes.from_hex("abc").__case, "None");
const b64 = c.bytes.to_base64(c.bytes.from_string("abc"));
assert.equal(b64, "YWJj");
assert.deepEqual([...c.bytes.from_base64(b64).value], [97, 98, 99]);
assert.equal(c.bytes.from_base64("!!").__case, "None");
assert.equal(c.bytes.from_base64("YQ").__case, "None"); // unpadded is invalid input
// unsafe: strict decode throws on invalid UTF-8 (no lossy substitution).
assert.throws(() => c.bytes.to_string(new Uint8Array([0xff])));
assert.equal(c.bytes.to_string(c.bytes.from_string("ok")), "ok");
// json + io + time.
assert.equal(c.json.validate("{{}}"), true);
assert.equal(c.json.validate("{{"), false);
assert.throws(() => c.json.parse("{{"));
assert.equal(typeof c.time.now(), "number");
// no ambient publication, nothing mutable.
assert.equal(typeof globalThis.__dekaCatalogBuild, "undefined");
assert.equal(globalThis.__dekaCatalog, undefined);
assert.ok(Object.isFrozen(c) && Object.isFrozen(c.bytes));
console.log("ok");
"#
        );
        let output = std::process::Command::new("node")
            .arg("-e")
            .arg(&script)
            .output()
            .expect("exec node");
        assert!(
            output.status.success(),
            "node behavior proof failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
