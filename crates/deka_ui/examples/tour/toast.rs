use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let visible = signal(false);
    view! {<view className="w-full h-full p-4 gap-3 bg-[#F3EFE3]">
        <button className="w-40 bg-[#663399] text-[#ffffff]" onClick={move |_| visible.toggle()}>"Toggle toast"</button>
        <div className="w-full h-52 overflow-hidden justify-end items-end bg-[#E2DCCF]">
            <div className={move || if visible.get().unwrap_or_default() {"w-64 p-4 gap-2 rounded-lg bg-[#0C8B43] text-[#ffffff] opacity-100 translate-y-0 transition-all duration-500 ease-out"} else {"w-64 p-4 gap-2 rounded-lg bg-[#0C8B43] text-[#ffffff] opacity-0 translate-y-24 transition-all duration-500 ease-out"}}>
                <span className="text-xl">"Changes saved"</span>
                <span>"A little motion makes the message easy to notice."</span>
            </div>
        </div>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
