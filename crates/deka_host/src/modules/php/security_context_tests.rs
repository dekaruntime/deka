//! Cross-thread isolation tests for the per-execution security context
//! (deka#725, Codex review of deka#729): a build-phase context installed on
//! one thread must not widen (or narrow) what enforcement on other threads
//! observes.

use super::security::enforce_read;

/// The build phase resolves the project policy (dev defaults can grant the
/// whole project root) and must execute under it WITHOUT exposing it to
/// concurrently served requests. A build context installed on one thread
/// widens only that thread; an ordinary request path on another thread keeps
/// the process policy's answer.
#[test]
fn build_phase_policy_is_not_observable_by_other_threads() {
    use runtime_core::security_context::{SecurityContext, set_security_context};

    let target = "data/secret.txt";
    // Baseline: whatever the process env grants, recorded before the build
    // phase starts (no env mutation happens in this test).
    let baseline_allowed = enforce_read(Some(target)).is_ok();

    let build_policy = r#"{"security":{"allow":{"read":["./"]},"deny":{},"prompt":false}}"#;
    let _guard = set_security_context(SecurityContext {
        policy_json: Some(build_policy.to_string()),
        no_prompt: true,
    });
    assert!(
        enforce_read(Some(target)).is_ok(),
        "the build-phase context must grant the read on its own thread"
    );
    let ordinary_allowed = std::thread::spawn(|| enforce_read(Some(target)).is_ok())
        .join()
        .expect("join");
    assert_eq!(
        ordinary_allowed, baseline_allowed,
        "a concurrent/ordinary request path must not gain the build-phase grant"
    );
}
