use deka_ui::{
    components::{Button, Theme, Toast},
    prelude::*,
};
use std::rc::Rc;

#[component]
pub fn App(#[prop(default = signal(Theme::Light))] theme: Signal<Theme>) -> View {
    let open = signal(true);
    let message = signal("Your changes are saved.".to_owned());
    view! {<view class={move || format!("w-full h-full p-6 gap-4 {}", theme.get().unwrap_or_default().page_class())}>
        <p class="text-2xl">"Toast"</p>
        <p>"A notification with an explicit lifetime."</p>
        <Button label="Show notification" theme={theme} on_press={Rc::new(move || open.set(true))}/>
        <Toast id="notification" message={message} open={open} theme={theme}/>
    </view>}
}
#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // The tour also imports this source as a module.
fn main() {
    deka_ui::launch(App);
}
