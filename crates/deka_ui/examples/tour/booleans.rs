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

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
