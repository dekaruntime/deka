use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let temperature = 12;
    let mut advice = "Enjoy the sunshine";
    if temperature < 15 {
        advice = "Bring a jacket";
    }
    view! {<view><p>"{advice}"</p></view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
