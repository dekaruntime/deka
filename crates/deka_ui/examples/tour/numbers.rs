use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let apples = 8;
    let pears = 4;
    view! {<view>
        <p>"Total fruit: "{apples + pears}</p>
        <p>"Apples left after eating two: "{apples - 2}</p>
        <p>"Twice as many pears: "{pears * 2}</p>
        <p>"Apples per person, shared by two: "{apples / 2}</p>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
