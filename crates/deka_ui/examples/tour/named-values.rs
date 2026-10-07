use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let name = "Sam";
    view! {<view><p>"Hello, {name}!"</p></view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
