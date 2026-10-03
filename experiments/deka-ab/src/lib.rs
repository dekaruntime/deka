//! deka A/B spike: the same deka_native_ui app on GPUI (backend A) and on
//! winit + wgpu + vello + parley (backend B).
pub mod common;
#[cfg(feature = "gpui-backend")]
pub mod gpui_backend;
#[cfg(feature = "vello-backend")]
pub mod vello_backend;
#[cfg(any(feature = "hybrid-backend", feature = "cpu-backend"))]
pub mod sparse;
