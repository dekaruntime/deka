use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    // This is a note for the person reading the code.
    // The computer skips these two lines.
    view! {<view><p>"My first program"</p></view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
