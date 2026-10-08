// Compile the production window module in a main-thread test executable.
// winit on macOS requires the process main thread; libtest workers cannot host it.
pub use deka_native_ui::*;
#[allow(dead_code)] // Only font context/startup helpers are used by the native event runner.
#[path = "../src/text.rs"]
mod text;
#[allow(dead_code, unused_imports)] // libtest helpers are compiled here but run in the separate library harness.
#[path = "../src/window/mod.rs"]
pub mod window;
fn main() {
    window::native_tests::run();
}
