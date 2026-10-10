use deka_ui::{
    components::{Button, Dialog, Input, Theme},
    prelude::*,
};
use std::rc::Rc;

#[component]
pub fn App(#[prop(default = signal(Theme::Light))] theme: Signal<Theme>) -> View {
    let open = signal(false);
    let name = signal("Sami".to_owned());
    view! {<view class={move || format!("w-full h-full p-6 gap-4 {}", theme.get().unwrap_or_default().page_class())}>
        <p class="text-2xl">"Dialog"</p>
        <p>"Focus stays inside. Escape closes and returns focus."</p>
        <Button label="Open dialog" theme={theme} on_press={Rc::new(move || open.set(true))}/>
        <Dialog id="profile-dialog" label="Edit profile" theme={theme} open={open}
            content={Rc::new(move || view! {<div class="gap-3"><Input label="Profile name" value={name} theme={theme}/><p>"Hello {name}"</p></div>})}/>
    </view>}
}
#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // The tour also imports this source as a module.
fn main() {
    deka_ui::launch(App);
}
