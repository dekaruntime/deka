use deka_ui::prelude::*;

#[component]
fn App() -> View {
    let mut count = signal(0);
    view! {
        <view className="p-4 gap-3">
            <p>"Count: {count}"</p>
            <button onClick={move |_| count += 1}>"Add one"</button>
        </view>
    }
}
fn main() {
    deka_ui::launch(App);
}
