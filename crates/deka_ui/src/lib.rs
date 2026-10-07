//! Rust UI authoring support for deka (APS 74).
//!
//! This initial slice supplies scoped reactive state. Retained View construction
//! and window launch will use the shared native tree after its extraction.
pub mod reactive;
pub use reactive::{Derived, Effect, Scope, Signal, batch, derived, effect, signal};

/// Common Rust UI authoring imports.
pub mod prelude {
    pub use crate::{Derived, Effect, Scope, Signal, batch, derived, effect, signal};
}
