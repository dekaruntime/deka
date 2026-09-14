//! One-shot updater for the cargo-via-universe distribution model, split
//! into submodules by concern (deka#976 / rfd#61 file-size gate -- this
//! file grew past the 1000-line cap once the deka#976 check-latest path
//! landed):
//!
//! - [`check`]: `deka --update`'s real check against the public release
//!   manifest (deka#976). Deliberately independent of `pipeline`.
//! - [`resolve`]: version parsing/comparison, and the linkhash-registry
//!   version lookup `pipeline::run_update` depends on.
//! - [`pipeline`]: the full build-on-old/snapshot/swap update flow and its
//!   CLI handler (`deka self update`). Resolves against a linkhash registry
//!   URL (default `http://localhost:9418`) that is the retired self-hosted
//!   registry and is not reachable in the current distribution model -- see
//!   CLAUDE.md's "Issue Tracking" / "Distribution" sections. This is NOT
//!   wired to `check`; do not treat it as live.

mod check;
mod pipeline;
mod resolve;

pub use check::check_latest;
pub(crate) use pipeline::validate_managed_unit_name;
pub use pipeline::{UpdateConfig, UpdateResult, cmd, get_registry_config, run_update};
pub use resolve::{LatestVersionInfo, resolve_latest_version};
