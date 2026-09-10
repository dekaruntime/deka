//! Cross-thread isolation tests for the per-execution security context
//! (deka#725, Codex review of deka#729): a build-phase context installed on
//! one thread must not widen (or narrow) what enforcement on other threads
//! observes.
//!
//! deka#801: the context is the ONLY policy channel. The tests below pin
//! the guarantee the issue exists for — the process environment cannot
//! influence what the isolation layer enforces.

use super::security::{enforce_net_public, enforce_read, security_policy_from_context};

/// Restores a process env var on drop. Scoped to one test; the values set
/// through it only ever name this test's own fixtures.
struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: String) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: single-process test binary; the guard restores the
        // previous value on drop, and no parallel test in this binary
        // reads the same key for its own assertions.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            // SAFETY: paired with the set_var in `set`, same process.
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

/// The point of deka#801: poisoning the process environment must not widen
/// what the isolation layer enforces. With `DEKA_SECURITY_POLICY` granting
/// everything, an installed restrictive policy still denies — and with no
/// context installed at all, enforcement errors instead of falling back to
/// the permissive environment.
#[test]
fn process_env_cannot_widen_the_installed_policy() {
    let _env = EnvGuard::set(
        "DEKA_SECURITY_POLICY",
        r#"{"security":{"allow":{"read":["/"],"net":["*"],"env":["*"]},"deny":{},"prompt":false}}"#
            .to_string(),
    );

    // 1. A permissive process env + a restrictive installed context: the
    //    context decides, so the read stays denied.
    let restrictive = r#"{"security":{"allow":{"read":["/nonexistent-deka-restrictive-canary"]},"deny":{},"prompt":false}}"#;
    let _guard = runtime_core::security_context::set_security_context(
        runtime_core::security_context::SecurityContext {
            policy_json: Some(restrictive.to_string()),
            no_prompt: true,
        },
    );
    assert!(
        enforce_read(Some("/etc/passwd")).is_err(),
        "a permissive DEKA_SECURITY_POLICY must not widen the installed policy"
    );
    assert!(
        enforce_net_public("api.example.com").is_err(),
        "a permissive DEKA_SECURITY_POLICY must not widen the installed net policy"
    );
    drop(_guard);

    // 2. A permissive process env and NO installed context: an error
    //    naming the missing dispatch, never the env policy.
    let err = security_policy_from_context()
        .expect_err("a missing context must be an error even under a permissive env");
    let message = err.to_string();
    assert!(
        message.contains("security context") || message.contains("dispatch"),
        "the diagnostic must name what failed to provide the policy: {message}"
    );
}

/// The build phase resolves the project policy (dev defaults can grant the
/// whole project root) and must execute under it WITHOUT exposing it to
/// concurrently served requests. A permissive build context installed on
/// one thread widens only that thread; an ordinary request context on
/// another thread keeps its own, narrower answer.
#[test]
fn build_phase_policy_is_not_observable_by_other_threads() {
    use runtime_core::security_context::{SecurityContext, set_security_context};

    let target = "data/secret.txt";
    let ordinary_policy =
        r#"{"security":{"allow":{"read":["/nonexistent-deka-ordinary-canary"]},"deny":{},"prompt":false}}"#;
    let build_policy = r#"{"security":{"allow":{"read":["./"]},"deny":{},"prompt":false}}"#;
    let deny_read = |policy: &'static str| {
        std::thread::spawn(move || {
            let _guard = set_security_context(SecurityContext {
                policy_json: Some(policy.to_string()),
                no_prompt: true,
            });
            enforce_read(Some(target)).is_err()
        })
        .join()
        .expect("join")
    };

    assert!(
        deny_read(ordinary_policy),
        "fixture check: the ordinary policy must deny {target}"
    );
    assert!(
        !deny_read(build_policy),
        "fixture check: the build-phase policy must grant {target}"
    );

    let _guard = set_security_context(SecurityContext {
        policy_json: Some(build_policy.to_string()),
        no_prompt: true,
    });
    assert!(
        enforce_read(Some(target)).is_ok(),
        "the build-phase context must grant the read on its own thread"
    );
    assert!(
        deny_read(ordinary_policy),
        "a concurrent/ordinary request path must not gain the build-phase grant"
    );
}
