//! One-shot updater for the cargo-via-universe distribution model, split
//! into submodules by concern (deka#976 / rfd#61 file-size gate; deka#990
//! moved `deka self update`'s handler onto the real check):
//!
//! - [`check`]: `deka self update`'s real handler (deka#976 / deka#990) --
//!   checks the public release manifest and reports plainly whether the
//!   running binary is current, or where the newer one is. Deliberately
//!   independent of `pipeline`.
//! - [`resolve`]: version parsing/comparison, and the linkhash-registry
//!   version lookup `pipeline::run_update` depends on.
//! - [`pipeline`]: the full build-on-old/snapshot/swap update flow. Still
//!   used by `deka self monitor` (see `monitor.rs`) via `run_update`, but
//!   its own CLI handler (`pipeline::cmd`) is no longer wired to any
//!   command -- deka#990 replaced it as `deka self update`'s handler with
//!   `check::cmd` because it resolved against a linkhash registry URL
//!   (default `http://localhost:9418`) that is the retired self-hosted
//!   registry and is not reachable in the current distribution model. See
//!   CLAUDE.md's "Issue Tracking" / "Distribution" sections, and
//!   `pipeline::cmd`'s own doc comment. This is NOT wired to `check`; do
//!   not treat it as live.

mod check;
mod pipeline;
mod resolve;

pub use check::{check_latest, cmd};
pub(crate) use pipeline::validate_managed_unit_name;
pub use pipeline::{UpdateConfig, UpdateResult, get_registry_config, run_update};
pub use resolve::{LatestVersionInfo, resolve_latest_version};
