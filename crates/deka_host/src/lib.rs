#![allow(clippy::all, dead_code, unused_imports)]

#[cfg(feature = "runtime")]
use deno_core::Extension;

pub mod integrity;
#[cfg(feature = "runtime")]
pub mod modules;
pub mod validation;

#[cfg(feature = "runtime")]
pub fn php_extension() -> Extension {
    modules::php::init()
}

#[cfg(feature = "runtime")]
pub fn extensions() -> Vec<Extension> {
    vec![modules::php::init()]
}

/// Build the host extensions with the net bridge pinned to an explicit
/// policy. Test-only counterpart of [`extensions`]: production callers
/// keep reading `DEKA_SECURITY_POLICY` from the env per dispatch.
#[cfg(feature = "runtime")]
pub fn extensions_with_net_policy(policy: SecurityPolicy) -> Vec<Extension> {
    vec![modules::php::init_with_net_policy(policy)]
}

#[cfg(feature = "runtime")]
pub use runtime_core::security_policy::SecurityPolicy;
