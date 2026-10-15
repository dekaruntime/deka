use deka_ui::{
    components::{Menu, MenuAction, Theme},
    prelude::*,
};

#[component]
pub fn App(#[prop(default = signal(Theme::Light))] theme: Signal<Theme>) -> View {
    let choice = signal("None".to_owned());
    let mut disabled = MenuAction::new("disabled", "Unavailable", || {});
    disabled.disabled = true;
    let items = vec![
        MenuAction::new("edit", "Edit", move || choice.set("Edit".into())),
        disabled,
        MenuAction::new("duplicate", "Duplicate", move || {
            choice.set("Duplicate".into())
        }),
    ];
    view! {<view class={move || format!("w-full h-full p-6 gap-4 {}", theme.get().unwrap_or_default().page_class())}>
        <p class="text-2xl">"Menu"</p>
        <p>"Use Up/Down, Home/End, Enter or Escape."</p>
        <Menu id="actions" label="Actions" items={items} theme={theme}/>
        <p>"Selected: {choice}"</p>
    </view>}
}
#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // The tour also imports this source as a module.
fn main() {
    deka_ui::launch(App);
}
