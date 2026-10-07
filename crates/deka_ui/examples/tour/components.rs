use deka_ui::prelude::*;

#[component]
fn Card(title: String, detail: String) -> View {
    view! {<div className="p-4 gap-2 rounded-lg bg-[#ffffff]"><p className="text-xl">"{title}"</p><p>"{detail}"</p></div>}
}

#[component]
pub fn App() -> View {
    view! {<view className="p-4 gap-3 bg-[#F3EFE3] text-[#1A1611]">
        <Card title="Deka" detail="A component is a function that returns a view."/>
        <Card title="Your project" detail="Reuse the structure with different arguments."/>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
