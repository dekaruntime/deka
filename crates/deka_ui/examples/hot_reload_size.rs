use deka_ui::prelude::*;
// DEKA_HOT_RELOAD_RELEASE_PAYLOAD_SENTINEL_1440
#[component]
fn Counter(label: String) -> View {
    let mut count = signal(0);
    view! {<view class="p-4"><button onClick={move |_| count+=1}>"Increment"</button><span>"{label}: {count}"</span></view>}
}
#[component]
fn App() -> View {
    view! {<Counter label="Value"/>}
}
fn main() {
    let app = UiApp::new(App);
    let _ = app.tree();
    assert!(app.dispatch(0));
    println!(
        "{}",
        app.tree().children[1]
            .children
            .iter()
            .filter_map(|node| node.text.as_deref())
            .collect::<String>()
    );
}
