//! Rust UI authoring support for deka (APS 74).
//!
//! Reactive Rust authoring on deka's shared retained tree and renderer.
extern crate self as deka_ui;
pub mod reactive;
#[cfg(feature = "tour")]
pub mod tour;
#[cfg(feature = "web")]
pub mod web;
pub use deka_ui_macros::{component, view};
pub use reactive::{Derived, Effect, ReactiveError, Scope, Signal, batch, derived, effect, signal};
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
mod desktop;
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
mod native_services;
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
pub use deka_native_ui::window::{DialogResult, FileDialogOptions};
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
pub use desktop::{AppClosed, DesktopApp, WindowHandle, WindowManager};
#[cfg(all(feature = "desktop", any(target_os = "macos", target_os = "windows")))]
pub use native_services::{Menu, MenuItem};
#[cfg(all(feature = "hot-reload", debug_assertions))]
pub mod hot_reload;
mod retained;
mod view;
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
pub use deka_native_ui::window::Options as WindowOptions;
pub use retained::{
    ComponentState, NodeRef, ViewClassList, ViewElement, ViewError, ViewNode, ViewTree, node_ref,
    tree,
};
pub use view::{Children, ErrorSink, Event, EventKind, UiApp, UiError, View};

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
#[doc(hidden)]
pub fn literal_string<T: From<&'static str>>(value: &'static str) -> T {
    value.into()
}
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use view::LaunchApp;
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
pub use view::{LaunchOptions, launch, launch_with, launch_with_options};

/// Common Rust UI authoring imports.
pub mod prelude {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    pub use crate::launch;
    pub use crate::{Children, Event, EventKind, UiApp, View};
    pub use crate::{
        ComponentState, NodeRef, ViewClassList, ViewElement, ViewError, ViewNode, ViewTree,
        node_ref, tree,
    };
    pub use crate::{
        Derived, Effect, ReactiveError, Scope, Signal, batch, derived, effect, signal,
    };
    #[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
    pub use crate::{
        DesktopApp, LaunchOptions, WindowHandle, WindowManager, WindowOptions, launch, launch_with,
        launch_with_options,
    };
    #[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
    pub use crate::{DialogResult, FileDialogOptions};
    #[cfg(all(feature = "desktop", any(target_os = "macos", target_os = "windows")))]
    pub use crate::{Menu, MenuItem};
    pub use crate::{component, view};
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub use web::launch;
