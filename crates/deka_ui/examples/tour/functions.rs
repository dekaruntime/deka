use deka_ui::prelude::*;

fn twice(value: i32) -> i32 {
    value * 2
}

#[component]
pub fn App() -> View {
    let mut value = signal(3);
    view! {<view class="p-6 gap-3 bg-[#F3EFE3] text-[#1A1611]">
        <p class="text-xl">"Functions calculate values"</p>
        <p>"Input: {value}"</p>
        <p>"Doubled: "{move || twice(value.get().unwrap_or_default())}</p>
        <button onClick={move |_| value += 1}>"Next number"</button>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
