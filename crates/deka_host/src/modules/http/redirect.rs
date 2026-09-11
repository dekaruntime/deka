//! Redirect handling for the `@deka/http` bridge (extracted from
//! `http.rs`, deka#391): every redirect hop re-runs the capability gate
//! so a 3xx from an allowlisted host to a non-allowlisted one cannot
//! carry Authorization / Cookie headers across origins.

use runtime_core::security_policy::SecurityPolicy;
use url::Url;

/// Returns the host portion of the URL in lowercase, with no port.
/// `None` if the URL can't be parsed.
fn host_of(url_str: &str) -> Option<String> {
    Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// Check a URL's host against the tenant's `net` allowlist. Returns
/// `Ok(())` when allowed, `Err(reason)` otherwise. Privileged code
/// (platform server / framework) is already exempted by the shared
/// `enforce_net` helper. The policy is passed in so callers can pin it
/// instead of re-resolving the per-execution security context per call
/// (deka#537).
pub(crate) fn enforce_host_allowed_with(
    policy: &SecurityPolicy,
    url_str: &str,
) -> Result<(), String> {
    let host = host_of(url_str).ok_or_else(|| format!("invalid url: '{}'", url_str))?;
    match crate::modules::php::enforce_net_public_with(policy, &host) {
        Ok(()) => Ok(()),
        Err(err) => Err(format!("host_not_allowed: {} ({})", host, err)),
    }
}

/// Build a redirect policy that re-runs the capability gate on every
/// hop. Fixes a capability-gate bypass where reqwest's default
/// `Policy::limited(N)` would follow 3xx across origins without
/// re-checking the host — a 302 from an allowlisted host to an
/// attacker host used to sail through, carrying Authorization /
/// Cookie headers with it.
///
/// `max_hops` caps the follow chain (matching the previous
/// `Policy::limited(N)` behaviour). When the next hop's host fails
/// the gate we `stop()` — reqwest surfaces this as a `redirect`
/// error which `encode_reqwest_error` maps to `too_many_redirects`;
/// we upgrade that to an explicit `host_not_allowed` classification
/// in the dispatch path by inspecting the error chain.
pub(super) fn host_checked_redirect_policy(
    policy: SecurityPolicy,
    max_hops: usize,
) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        // Snapshot what we need from `attempt` before any terminal
        // call consumes it — `attempt.error()` / `attempt.follow()`
        // / `attempt.stop()` all take `self` by value.
        let host = attempt.url().host_str().unwrap_or("").to_ascii_lowercase();
        let url = attempt.url().to_string();

        if attempt.previous().len() >= max_hops {
            return attempt.error(RedirectHostDenied {
                host,
                reason: format!("too many redirects (> {})", max_hops),
            });
        }
        if let Err(err) = enforce_host_allowed_with(&policy, &url) {
            return attempt.error(RedirectHostDenied { host, reason: err });
        }
        attempt.follow()
    })
}

/// Marker error type surfaced by `host_checked_redirect_policy` when a
/// redirect hop is refused. reqwest wraps this inside its own
/// `reqwest::Error` (kind = redirect); we walk the source chain in
/// `encode_reqwest_error` to detect it and promote the response
/// classification from the generic `too_many_redirects` to
/// `host_not_allowed`, which matches what the initial-URL gate
/// already returns for a disallowed host.
#[derive(Debug)]
pub(super) struct RedirectHostDenied {
    pub(super) host: String,
    pub(super) reason: String,
}

impl std::fmt::Display for RedirectHostDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "redirect host not allowed: {} ({})",
            self.host, self.reason
        )
    }
}

impl std::error::Error for RedirectHostDenied {}
