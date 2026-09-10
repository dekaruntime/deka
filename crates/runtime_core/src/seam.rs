//! The `seam.contract@1` document model.
//!
//! The types live in the `seam_ir` crate so that `seam_diff`, `seam_ts` and the
//! `deka contract-extract` / `contract-check` commands share one definition
//! rather than each carrying a copy. This module is the re-export the
//! runtime-side producer contracts (`storefront_envelope`, `data_envelope`,
//! `platform_env`) were already written against, so those files need no edits.
//! Recovered in deka #789.

pub use seam_ir::*;
