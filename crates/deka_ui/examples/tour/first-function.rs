use deka_ui::prelude::*;

fn double(value: i32) -> i32 {
    value * 2
}

#[component]
pub fn App() -> View {
    let answer = double(3);
    view! {<view><p>"The answer is {answer}"</p></view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
