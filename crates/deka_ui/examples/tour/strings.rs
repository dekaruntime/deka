use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let first = "Sam";
    let last = "River";
    let full_name = format!("{first} {last}");
    view! {<view><p>"{full_name}"</p><p>"Welcome to your app!"</p></view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
