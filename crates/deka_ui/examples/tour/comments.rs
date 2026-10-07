use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    // This is a note for the person reading the code.
    // The computer skips these two lines.
    view! {<view><p>"My first program"</p></view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
