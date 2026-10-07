//! The tour contains compiled Rust lessons, no compiler or interpreter.
#[cfg(target_arch = "wasm32")]
mod browser {
    use deka_ui::{
        tour,
        web::{WebHandle, mount_app},
    };
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    thread_local! { static RUNNING: RefCell<Option<WebHandle>> = const { RefCell::new(None) }; }
    /// Replace the current lesson without loading a second wasm instance.
    #[wasm_bindgen]
    pub fn start(lesson_id: &str, canvas: &JsValue) -> Result<(), JsValue> {
        #[cfg(feature = "counter-only")]
        let app = {
            if lesson_id != "counter" {
                return Err(JsValue::from_str("Unknown lesson"));
            }
            deka_ui::UiApp::new(tour::counter::App)
        };
        #[cfg(all(not(feature = "counter-only"), feature = "largest-only"))]
        let app = {
            if lesson_id != "layout" {
                return Err(JsValue::from_str("Unknown lesson"));
            }
            deka_ui::UiApp::new(tour::layout::App)
        };
        #[cfg(not(any(feature = "counter-only", feature = "largest-only")))]
        let app = (tour::LESSONS
            .iter()
            .find(|l| l.id == lesson_id)
            .ok_or_else(|| JsValue::from_str("Unknown lesson"))?
            .app)();
        stop();
        let handle = mount_app(app, canvas)?;
        RUNNING.with(|running| running.replace(Some(handle)));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn stop() {
        RUNNING.with(|running| running.replace(None));
    }
}
