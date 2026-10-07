use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    view! {<view><p>"Hello, world!"</p></view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
