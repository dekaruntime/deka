use deka_ui::{
    components::{Button, Input, Theme},
    prelude::*,
};
use std::rc::Rc;

#[component]
pub fn App(#[prop(default = signal(Theme::Light))] theme: Signal<Theme>) -> View {
    let name = signal("Sami".to_owned());
    view! { <view class={move || format!("w-full h-full p-6 gap-4 {}", theme.get().unwrap_or_default().page_class())}>
        <p class="text-2xl">"Input"</p>
        <p>"Name"</p>
        <Input id="name" label="Name" placeholder="Your name" value={name} theme={theme}/>
        <p>"Hello {name}"</p>
        <Button label="Clear" theme={theme} on_press={Rc::new(move || name.set(String::new()))}/>
    </view> }
}
#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // The tour also imports this source as a module.
fn main() {
    deka_ui::launch(App);
}
