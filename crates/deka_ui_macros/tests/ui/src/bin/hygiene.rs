use deka_ui::prelude::*;
fn __deka_props() -> &'static str { "Global" }
#[component]
fn UsesGlobal(title: String) -> View {
    let global = __deka_props();
    view! { <p>"{global} {title}"</p> }
}
mod props {
    use deka_ui::prelude::*;
    #[component]
    pub fn UsesProp(__deka_props: String) -> View {
        let __deka_props = format!("{__deka_props}:local");
        view! { <p>{__deka_props}</p> }
    }
}
fn main() {
    let app = UiApp::new(|| view! { <view>
        <UsesGlobal title="Title" />
        <props::UsesProp __deka_props="Value" />
    </view> });
    let tree = app.tree();
    let text = tree.children.iter().flat_map(|node| node.children.iter())
        .filter_map(|node| node.text.as_deref()).collect::<Vec<_>>();
    println!("{}", text.join(" ").split_whitespace().collect::<Vec<_>>().join(" "));
}
