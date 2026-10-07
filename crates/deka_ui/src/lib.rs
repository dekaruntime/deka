//! Rust UI authoring support for deka (APS 74).
//!
//! Reactive Rust authoring on deka's shared retained tree and renderer.
pub mod reactive;
pub use deka_ui_macros::{component, view};
pub use reactive::{Derived, Effect, ReactiveError, Scope, Signal, batch, derived, effect, signal};
mod view;
#[cfg(feature = "desktop")]
pub use deka_native_ui::window::Options as WindowOptions;
pub use view::{Children, Event, EventKind, UiApp, View};

/// Generated component props provide their typed builder. Macro expansion
/// infers it from the imported Rust function rather than a second name lookup.
#[doc(hidden)]
pub trait ComponentProps {
    type Builder;
    fn builder() -> Self::Builder;
}
#[doc(hidden)]
pub fn props_for<F, P>(_: F) -> P::Builder
where
    F: FnOnce(P) -> View,
    P: ComponentProps,
{
    P::builder()
}
#[cfg(feature = "desktop")]
pub use view::{launch, launch_with};

/// Common Rust UI authoring imports.
pub mod prelude {
    pub use crate::{Children, Event, EventKind, UiApp, View};
    pub use crate::{
        Derived, Effect, ReactiveError, Scope, Signal, batch, derived, effect, signal,
    };
    #[cfg(feature = "desktop")]
    pub use crate::{WindowOptions, launch, launch_with};
    pub use crate::{component, view};
}
