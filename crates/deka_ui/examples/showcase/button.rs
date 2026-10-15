use deka_ui::{
    components::{Button, Theme},
    prelude::*,
};
use std::rc::Rc;

#[component]
pub fn App(#[prop(default = signal(Theme::Light))] theme: Signal<Theme>) -> View {
    let count = signal(0);
    view! { <view class={move || format!("w-full h-full p-6 gap-4 {}", theme.get().unwrap_or_default().page_class())}>
        <p class="text-2xl">"Button"</p>
        <p>"Activate with a click, Enter or Space."</p>
        <Button id="add" label="Add one" theme={theme} on_press={Rc::new(move || { count.update(|n| *n += 1).unwrap(); })}/>
        <p>"Count: {count}"</p>
        <Button id="disabled" label="Unavailable" theme={theme} disabled={signal(true)} on_press={Rc::new(|| {})}/>
    </view> }
}
#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // The tour also imports this source as a module.
fn main() {
    deka_ui::launch(App);
}
