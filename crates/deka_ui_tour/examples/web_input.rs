//! Browser integration fixture: native editing feeds typed Rust events.
#[cfg(target_arch = "wasm32")]
mod web {
    use deka_ui::prelude::*;
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    thread_local! { static APP: RefCell<Option<deka_ui::web::WebHandle>> = const { RefCell::new(None) }; }
    #[component]
    fn App() -> View {
        let text = signal(String::new());
        let key = signal(String::new());
        view! {<view className="p-4 gap-4">
            <input id="name" className="w-60 h-10" value={text}
                onKeyDown={move |event| if let Event::KeyDown(value)=event { key.set(value.clone()); if value=="Enter" { text.set("Confirmed".to_owned()); } }}/>
            <textarea id="notes" className="w-60 h-24" value={text}/>
            <p>"Hello {text}"</p><p>"Key: {key}"</p>
        </view>}
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
