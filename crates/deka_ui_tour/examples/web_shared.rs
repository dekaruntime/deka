//! Independently owned browser mounts sharing one reactive scope.
#[cfg(target_arch = "wasm32")]
mod web {
    use deka_ui::{
        prelude::*,
        web::{WebHandle, mount_app},
    };
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    thread_local! { static APPS: RefCell<Vec<WebHandle>> = const {RefCell::new(Vec::new())}; }
    #[wasm_bindgen]
    pub fn start(first: &JsValue, second: &JsValue) -> Result<(), JsValue> {
        stop();
        let scope = Scope::new();
        let mut count = scope.run(|| signal(0));
        for (name, canvas) in [("First", first), ("Second", second)] {
            let app = UiApp::new_in_scope(&scope, move || {
                view! {
                    <view class="p-4 gap-4"><p>"{name}: {count}"</p>
                    <button onClick={move |_| count += 1}>"Add"</button></view>
                }
            });
            let handle = mount_app(app, canvas)?;
            APPS.with(|apps| apps.borrow_mut().push(handle));
        }
        Ok(())
    }
    #[wasm_bindgen]
    pub fn close_first() {
        APPS.with(|apps| {
            apps.borrow_mut().remove(0);
        });
    }
    #[wasm_bindgen]
    pub fn stop() {
        APPS.with(|apps| apps.borrow_mut().clear());
    }
}
