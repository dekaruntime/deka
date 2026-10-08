use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view class="w-full h-full p-4 gap-4 bg-[#F3EFE3]">
        <button class="w-48 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle scale and rotation"</button>
        <span>"The hit area moves with the transformed button."</span>
        <div class="w-full h-64 p-8 gap-3 overflow-hidden">
            <button class={move || if open.get().unwrap_or_default() {"w-32 h-20 rounded-lg bg-[#663399] text-[#ffffff] scale-125 rotate-20 transition-transform duration-700"} else {"w-32 h-20 rounded-lg bg-[#663399] text-[#ffffff] scale-100 rotate-0 transition-transform duration-700"}}
                onClick={move |_| open.toggle()}>"Click me too"</button>
        </div>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
