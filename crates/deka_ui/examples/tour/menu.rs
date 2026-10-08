use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view class="w-full h-full p-4 gap-3 bg-[#F3EFE3]">
        <button class="w-40 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle menu"</button>
        <div class="w-full h-52 overflow-hidden bg-[#E2DCCF]">
            <div class={move || if open.get().unwrap_or_default() {"w-56 h-full p-4 gap-4 bg-[#663399] text-[#ffffff] translate-x-0 transition-transform duration-500 ease-out"} else {"w-56 h-full p-4 gap-4 bg-[#663399] text-[#ffffff] -translate-x-56 transition-transform duration-500 ease-out"}}>
                <span class="text-xl">"Your workspace"</span>
                <span>"Projects"</span>
                <span>"Activity"</span>
                <button class="bg-[#ffffff] text-[#663399]" onClick={move |_| open.set(false)}>"Close menu"</button>
            </div>
        </div>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
