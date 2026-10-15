use deka_ui::{
    components::{Button, List, ListItem, Theme},
    prelude::*,
};
use std::rc::Rc;

#[component]
pub fn App(#[prop(default = signal(Theme::Light))] theme: Signal<Theme>) -> View {
    let projects = signal(vec![
        ListItem::new("deka", "Deka"),
        ListItem::new("zega", "Zega"),
        ListItem::new("cqx", "cqx"),
    ]);
    let selected = signal(Some("deka".to_owned()));
    view! { <view class={move || format!("w-full h-full p-6 gap-4 {}", theme.get().unwrap_or_default().page_class())}>
        <p class="text-2xl">"Keyed List"</p>
        <p>"Choose a project. Reordering keeps its identity and focus."</p>
        <List id="projects" label="Projects" items={projects} selected={selected} theme={theme}/>
        <p>{move || format!("Selected: {}", selected.get().unwrap_or_default().unwrap_or_default())}</p>
        <Button label="Reverse order" theme={theme} on_press={Rc::new(move || { projects.update(|items| items.reverse()).unwrap(); })}/>
    </view> }
}
#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // The tour also imports this source as a module.
fn main() {
    deka_ui::launch(App);
}
