use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view class="w-full h-full p-4 gap-4 bg-[#F3EFE3]">
        <button class="w-48 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle layout movement"</button>
        <span>"Changing alignment gives the box a new layout destination."</span>
        <div class="w-full h-64 p-8 gap-3 overflow-hidden">
            <div class={move || if open.get().unwrap_or_default() {"w-full h-28 flex-row justify-end bg-[#E2DCCF]"} else {"w-full h-28 flex-row justify-start bg-[#E2DCCF]"}}>
                <div class="w-20 h-20 shrink-0 rounded-lg bg-[#0C8B43] transition-layout spring"/>
            </div>
        </div>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
