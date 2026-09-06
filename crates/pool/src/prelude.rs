//! Isolate bootstrap constructors for `Result` / `Option` (deka#582).
//!
//! dsc owns the compiler-side prelude (`module_prelude`, emit helpers). The
//! pool still has to install the same branded shapes on `globalThis` so host
//! bridges and user code agree. These strings are the host copy of that ABI
//! and must stay byte-identical to dsc's pool prelude / to_result helper.

const RESULT_OK: &str =
    r#"(value) => ({ __enum: "Result", __case: "Ok", name: "Ok", value })"#;
const RESULT_ERR: &str =
    r#"(error) => ({ __enum: "Result", __case: "Err", name: "Err", error })"#;
const OPTION_SOME: &str =
    r#"(value) => ({ __enum: "Option", __case: "Some", name: "Some", value })"#;
const OPTION_NONE: &str = r#"({ __enum: "Option", __case: "None", name: "None" })"#;

/// Pool bootstrap prelude: constructors on `globalThis` behind `typeof`
/// guards so user code and repeated bootstraps cannot clobber them.
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
/// expressions as the prelude.
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
    fn constructors_are_shared_across_sites() {
        let pool = pool_prelude();
        let to_result = to_result_helper();
        for ctor in [RESULT_OK, RESULT_ERR, OPTION_SOME, OPTION_NONE] {
            assert!(pool.contains(ctor), "pool prelude missing {ctor}");
        }
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
        for site in [pool_prelude(), to_result_helper()] {
            assert!(
                !site.contains("Object.freeze({ __enum:"),
                "ephemeral enum values must not be frozen (rfd#13): {site}"
            );
        }
        assert!(pool_prelude().contains("globalThis.Result = Object.freeze({"));
    }
}
