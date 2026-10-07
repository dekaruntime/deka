use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let fruits = ["Apple", "Pear", "Orange"];
    view! {<view>
        <p>"First fruit: "{fruits.first().copied().unwrap_or("No fruit")}</p>
        <p>"Second fruit: "{fruits.get(1).copied().unwrap_or("No fruit")}</p>
        <p>"Third fruit: "{fruits.get(2).copied().unwrap_or("No fruit")}</p>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
