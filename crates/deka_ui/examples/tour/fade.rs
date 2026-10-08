use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view class="w-full h-full p-4 gap-3 bg-[#F3EFE3]">
        <button class="w-40 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle modal"</button>
        <div class="w-full h-56 shrink-0 items-center justify-center overflow-hidden bg-[#E2DCCF]">
            <div class={move || if open.get().unwrap_or_default() {"w-64 p-4 gap-3 rounded-lg bg-[#ffffff] opacity-100 transition-opacity duration-500 ease-out"} else {"w-64 p-4 gap-3 rounded-lg bg-[#ffffff] opacity-0 transition-opacity duration-500 ease-out"}}>
                <span class="text-xl">"A quiet entrance"</span>
                <span>"This card fades in and out. Click again midway to reverse it."</span>
                <button class="bg-[#663399] text-[#ffffff]" onClick={move |_| open.set(false)}>"Dismiss"</button>
            </div>
        </div>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
