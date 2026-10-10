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
        let app =
            tour::app(lesson_id).ok_or_else(|| JsValue::from_str("Unknown lesson or component"))?;
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
