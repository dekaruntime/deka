use deka_ui::prelude::*;

fn double(value: i32) -> i32 {
    value * 2
}

#[component]
pub fn App() -> View {
    let answer = double(3);
    view! {<view><p>"The answer is {answer}"</p></view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
