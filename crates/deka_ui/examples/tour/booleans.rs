use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let lights_on = true;
    let message = if lights_on {
        "The lights are on"
    } else {
        "The lights are off"
    };
    view! {<view><p>"{message}"</p></view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
