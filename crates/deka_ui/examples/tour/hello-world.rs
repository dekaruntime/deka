use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    view! {<view><p>"Hello, world!"</p></view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
