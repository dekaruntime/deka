//! Per-execution security context (deka#725).
//!
//! The serve/run paths export the resolved security policy through
//! `DEKA_SECURITY_POLICY` / `DEKA_SECURITY_NO_PROMPT`, which bridge
//! enforcement reads per call. Build-slot materialization must execute under
//! the project's policy WITHOUT mutating that process-wide state: under
//! `deka dev` the server keeps serving requests on other threads, and those
//! requests would observe the build-phase policy (Codex review of deka#729).
//!
//! Instead, an execution that needs a different policy (build slots) or
//! prompt suppression installs a [`SecurityContext`] on its own thread for
//! the duration of the execution; enforcement prefers it over the env vars.
//! Isolate workers run ops on the thread that installs the context, so the
//! guard in `pool::WorkerThread::process_request` covers every bridge call
//! of that execution.

use std::cell::RefCell;

/// Security policy overrides for one execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecurityContext {
    /// Resolved policy JSON (the same payload `DEKA_SECURITY_POLICY` carries).
    /// `None` means "no override" — enforcement falls back to the env var.
    pub policy_json: Option<String>,
    /// Suppress interactive approval prompts for this execution (the build
    /// phase must fail, not block, on a denied capability).
    pub no_prompt: bool,
}

thread_local! {
    static CURRENT: RefCell<Option<SecurityContext>> = const { RefCell::new(None) };
}

/// Restores the previous context (usually `None`) on drop.
pub struct SecurityContextGuard {
    previous: Option<SecurityContext>,
}

impl Drop for SecurityContextGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            *current.borrow_mut() = self.previous.take();
        });
    }
}

/// Install `context` as the current thread's security context until the
/// returned guard drops. Nesting restores the outer context.
pub fn set_security_context(context: SecurityContext) -> SecurityContextGuard {
    let previous = CURRENT.with(|current| current.borrow_mut().replace(context));
    SecurityContextGuard { previous }
}

/// The current thread's security context, if any.
pub fn current_security_context() -> Option<SecurityContext> {
    CURRENT.with(|current| current.borrow().clone())
}

/// The policy JSON installed for the current thread's execution, if any.
/// Enforcement readers prefer this over the `DEKA_SECURITY_POLICY` env var.
pub fn context_policy_json() -> Option<String> {
    current_security_context().and_then(|context| context.policy_json)
}

/// Whether the current execution suppresses interactive approval prompts.
pub fn context_no_prompt() -> bool {
    current_security_context().is_some_and(|context| context.no_prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_is_scoped_to_the_installing_thread_and_guard() {
        let outer = current_security_context();
        let context = SecurityContext {
            policy_json: Some("{\"allow\":{}}".to_string()),
            no_prompt: true,
        };
        let installed = context.clone();
        {
            let _guard = set_security_context(context);
            assert_eq!(current_security_context(), Some(installed.clone()));
            // A thread without an installation must not observe the context,
            // regardless of what the process env grants.
            let other = std::thread::spawn(current_security_context)
                .join()
                .expect("join");
            assert_eq!(other, outer, "context leaked across threads");
        }
        assert_eq!(current_security_context(), outer, "guard must restore");
    }

    #[test]
    fn nested_contexts_restore_in_order() {
        let first = SecurityContext {
            policy_json: Some("{\"first\":true}".to_string()),
            no_prompt: false,
        };
        let second = SecurityContext {
            policy_json: Some("{\"second\":true}".to_string()),
            no_prompt: true,
        };
        let _outer = set_security_context(first.clone());
        assert_eq!(current_security_context(), Some(first.clone()));
        {
            let _inner = set_security_context(second.clone());
            assert_eq!(current_security_context(), Some(second));
        }
        assert_eq!(current_security_context(), Some(first.clone()));
    }
}
