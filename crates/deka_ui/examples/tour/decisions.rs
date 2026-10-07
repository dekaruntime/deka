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

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
