use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let mut count = signal(0);
    view! {<view class="flex flex-col gap-4 p-6 bg-[#F3EFE3] text-[#1A1611]">
        <span class="text-2xl">"Deka, native."</span>
        <span>"The same UI, inside your browser."</span>
        <div class="flex flex-row gap-4">
            <button class="p-4 rounded-lg bg-[#0C8B43] text-[#ffffff]" onClick={move |_| count += 1}>"Add one"</button>
            <button class="p-4 rounded-lg bg-[#5946AD] text-[#ffffff]" onClick={move |_| count -= 1}>"Subtract one"</button>
        </div>
        <span class="text-xl">"Count: {count}"</span>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
