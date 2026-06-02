#[cfg(all(feature = "macos", target_os = "macos"))]
mod macos;
#[cfg(not(all(feature = "macos", target_os = "macos")))]
mod stub;

#[cfg(all(feature = "macos", target_os = "macos"))]
pub use macos::VirtualizationFrameworkBackend;
#[cfg(not(all(feature = "macos", target_os = "macos")))]
pub use stub::VirtualizationFrameworkBackend;
