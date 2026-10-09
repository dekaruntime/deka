//! Browser development fixture; the real host paints and dispatches input.
// DEKA_HOT_RELOAD_RELEASE_PAYLOAD_SENTINEL_1440
#[cfg(target_arch = "wasm32")]
mod web {
    use deka_ui::prelude::*;
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    thread_local! { static APP: RefCell<Option<deka_ui::web::WebHandle>> = const { RefCell::new(None) }; }
    #[component]
    fn Counter(title: String, seed: i32) -> View {
        let mut count = signal(seed);
        view! {<button id="counter" class="p-4 bg-[#5946AD]" onClick={move |_| count += 1}>"{title}: {count}"</button>}
    }
    #[component]
    fn App() -> View {
        let show = signal(false);
        let conditional = move || show.get().unwrap().then(|| view! {<p>"Conditional"</p>});
        view! {<view class="p-4 gap-4"><p>"Before"</p><div id="left"><Counter title="Count" seed=0/>{conditional}</div><div id="right"/><button id="toggle" onClick={move |_| show.update(|v| *v = !*v).unwrap()}>"Toggle"</button></view>}
    }
    #[wasm_bindgen]
    pub fn start(canvas: &JsValue) -> Result<(), JsValue> {
        stop();
        let handle = deka_ui::launch(App, canvas)?;
        APP.with(|app| app.replace(Some(handle)));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn stop() {
        APP.with(|app| app.replace(None));
    }
}
