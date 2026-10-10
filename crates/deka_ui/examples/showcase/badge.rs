use deka_ui::{
    components::{Badge, Button, Theme},
    prelude::*,
};
use std::rc::Rc;

#[component]
pub fn App(#[prop(default = signal(Theme::Light))] theme: Signal<Theme>) -> View {
    let status = signal("Ready".to_owned());
    view! {<view class={move || format!("w-full h-full p-6 gap-4 {}", theme.get().unwrap_or_default().page_class())}>
        <p class="text-2xl">"Badge"</p>
        <p>"Live status without an extra focus stop."</p>
        <Badge id="status" label={status} theme={theme}/>
        <Button label="Update status" theme={theme} on_press={Rc::new(move || status.set("Published".into()))}/>
    </view>}
}
#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // The tour also imports this source as a module.
fn main() {
    deka_ui::launch(App);
}
