//! Single source of truth for the shared enum runtime prelude
//! (`Result`/`Option` constructors), consumed by every construction site
//! (deka#582):
//!
//! - [`module_prelude`] — the prelude emitted into compiled modules.
//! - [`pool_prelude`] — the `globalThis`-guarded form injected by the
//!   isolate pool bootstrap (`crates/pool/src/isolate_pool/worker_execution.rs`).
//! - [`to_result_helper`] — the `__deka_to_result` expression (deka#578)
//!   that tags host-bridge envelopes with the same shapes.
//!
//! Freezing rules follow rfd#13 principles 1 and 8: the namespace objects
//! (`Result`, `Option`) are shared and long-lived, so they stay frozen;
//! ephemeral `Ok(v)`/`Some(v)` values carry data, not guarantees, and are
//! never frozen.

const RESULT_OK: &str = r#"(value) => ({ __enum: "Result", __case: "Ok", name: "Ok", value })"#;
const RESULT_ERR: &str = r#"(error) => ({ __enum: "Result", __case: "Err", name: "Err", error })"#;
const OPTION_SOME: &str = r#"(value) => ({ __enum: "Option", __case: "Some", name: "Some", value })"#;
const OPTION_NONE: &str = r#"({ __enum: "Option", __case: "None", name: "None" })"#;

/// Module-local prelude emitted into compiled modules: frozen namespace
/// consts plus the bare `Ok`/`Err`/`Some`/`None` aliases the emitter
/// rewrites references to.
pub fn module_prelude() -> String {
    format!(
        "const Result = Object.freeze({{\n\
        \x20 Ok: {RESULT_OK},\n\
        \x20 Err: {RESULT_ERR}\n\
        }});\n\
        const Option = Object.freeze({{\n\
        \x20 Some: {OPTION_SOME},\n\
        \x20 None: {OPTION_NONE}\n\
        }});\n\
        const Ok = Result.Ok;\n\
        const Err = Result.Err;\n\
        const Some = Option.Some;\n\
        const None = Option.None;\n"
    )
}

/// Pool bootstrap prelude: the same constructors, installed on `globalThis`
/// behind `typeof` guards so user code and repeated bootstraps cannot
/// clobber them.
pub fn pool_prelude() -> String {
    format!(
        "if (typeof globalThis.Option === 'undefined') {{\n\
        \x20   globalThis.Option = Object.freeze({{\n\
        \x20       Some: {OPTION_SOME},\n\
        \x20       None: {OPTION_NONE}\n\
        \x20   }});\n\
        }}\n\
        if (typeof globalThis.Result === 'undefined') {{\n\
        \x20   globalThis.Result = Object.freeze({{\n\
        \x20       Ok: {RESULT_OK},\n\
        \x20       Err: {RESULT_ERR}\n\
        \x20   }});\n\
        }}\n"
    )
}

/// `__deka_to_result` (deka#578): normalize a `{ ok, value | error }`
/// bridge envelope into a tagged `Result`. Built from the same constructor
/// expressions as the prelude, so the envelope shape can never drift from
/// what `Result.Ok`/`Result.Err` produce.
pub fn to_result_helper() -> String {
    format!(
        "(r) => (r && r.ok)\n\
        \x20   ? ({RESULT_OK})(r.value)\n\
        \x20   : ({RESULT_ERR})((r && r.error) ? r.error : \"host bridge failed\")"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_prelude_matches_emitter_contract() {
        // Byte-exact contract asserted by the emitter's existing tests and
        // snapshots; drift here means drift in compiled output.
        assert_eq!(
            module_prelude(),
            concat!(
                "const Result = Object.freeze({\n",
                "  Ok: (value) => ({ __enum: \"Result\", __case: \"Ok\", name: \"Ok\", value }),\n",
                "  Err: (error) => ({ __enum: \"Result\", __case: \"Err\", name: \"Err\", error })\n",
                "});\n",
                "const Option = Object.freeze({\n",
                "  Some: (value) => ({ __enum: \"Option\", __case: \"Some\", name: \"Some\", value }),\n",
                "  None: ({ __enum: \"Option\", __case: \"None\", name: \"None\" })\n",
                "});\n",
                "const Ok = Result.Ok;\n",
                "const Err = Result.Err;\n",
                "const Some = Option.Some;\n",
                "const None = Option.None;\n",
            )
        );
    }

    #[test]
    fn all_sites_share_constructor_shapes() {
        let module = module_prelude();
        let pool = pool_prelude();
        let to_result = to_result_helper();
        for ctor in [RESULT_OK, RESULT_ERR, OPTION_SOME, OPTION_NONE] {
            assert!(module.contains(ctor), "module prelude missing {ctor}");
            // `None` is a module-only alias target; the pool installs the
            // namespaced table that the same constructor defines.
            assert!(pool.contains(ctor), "pool prelude missing {ctor}");
        }
        // The bridge helper calls the Result constructors rather than
        // re-transcribing the tagged literals: the only `__enum` mentions
        // are the ones inside the shared constructor expressions.
        assert!(to_result.contains(RESULT_OK), "to_result missing Ok ctor");
        assert!(to_result.contains(RESULT_ERR), "to_result missing Err ctor");
        assert_eq!(
            to_result.matches("__enum").count(),
            RESULT_OK.matches("__enum").count() + RESULT_ERR.matches("__enum").count(),
            "to_result must reuse constructors, not re-transcribe literals: {to_result}"
        );
    }

    #[test]
    fn ephemeral_values_are_never_frozen() {
        for site in [module_prelude(), pool_prelude(), to_result_helper()] {
            assert!(
                !site.contains("Object.freeze({ __enum:"),
                "ephemeral enum values must not be frozen (rfd#13): {site}"
            );
        }
        // The long-lived namespace tables stay frozen.
        assert!(module_prelude().contains("const Result = Object.freeze({"));
        assert!(pool_prelude().contains("globalThis.Result = Object.freeze({"));
    }
}
