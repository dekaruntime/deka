use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let large = signal(false);
    view! {<view class="w-full h-full p-4 gap-4 bg-[#F3EFE3] items-start">
        <span class="text-xl">"Make room"</span>
        <button class={move || if large.get().unwrap_or_default() {"w-64 h-20 bg-[#0C8B43] text-[#ffffff] transition-all duration-600 ease-in-out"} else {"w-40 h-12 bg-[#663399] text-[#ffffff] transition-all duration-600 ease-in-out"}}
            onClick={move |_| large.toggle()}>"Click to resize"</button>
        <span class="w-64">"The next box moves as the button grows. This is animated layout, not a stretched image."</span>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
