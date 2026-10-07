use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let first = "Sam";
    let last = "River";
    let full_name = format!("{first} {last}");
    view! {<view><p>"{full_name}"</p><p>"Welcome to your app!"</p></view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
