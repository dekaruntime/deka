use deka_ui::{
    components::{TabItem, Tabs, Theme},
    prelude::*,
};

#[component]
pub fn App(#[prop(default = signal(Theme::Light))] theme: Signal<Theme>) -> View {
    let selected = signal("overview".to_owned());
    let tabs = vec![
        TabItem::new(
            "overview",
            "Overview",
            || view! { <p>"Everything you need in one place."</p> },
        ),
        TabItem::new(
            "activity",
            "Activity",
            || view! { <p>"Your latest project activity."</p> },
        ),
        TabItem::new(
            "settings",
            "Settings",
            || view! { <p>"Make this workspace your own."</p> },
        ),
    ];
    view! { <view class={move || format!("w-full h-full p-6 gap-4 {}", theme.get().unwrap_or_default().page_class())}>
        <p class="text-2xl">"Tabs"</p>
        <p>"Use Left, Right, Home or End to choose a tab."</p>
        <Tabs id="workspace" label="Workspace" items={tabs} selected={selected} theme={theme}/>
        <p>"Selected: {selected}"</p>
    </view> }
}
#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // The tour also imports this source as a module.
fn main() {
    deka_ui::launch(App);
}
