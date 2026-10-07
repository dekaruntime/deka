//! Rust UI authoring support for deka (APS 74).
//!
//! Reactive Rust authoring on deka's shared retained tree and renderer.
pub mod reactive;
pub use reactive::{Derived, Effect, Scope, Signal, batch, derived, effect, signal};
mod view;
#[cfg(feature = "desktop")]
pub use deka_native_ui::window::Options as WindowOptions;
pub use view::{Children, Event, EventKind, UiApp, View};
#[cfg(feature = "desktop")]
pub use view::{launch, launch_with};

/// Common Rust UI authoring imports.
pub mod prelude {
    pub use crate::{Children, Event, EventKind, UiApp, View};
    pub use crate::{Derived, Effect, Scope, Signal, batch, derived, effect, signal};
    #[cfg(feature = "desktop")]
    pub use crate::{WindowOptions, launch, launch_with};
}
