use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view className="w-full h-full p-4 gap-3 bg-[#F3EFE3]">
        <button className="w-40 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle menu"</button>
        <div className="w-full h-52 overflow-hidden bg-[#E2DCCF]">
            <div className={move || if open.get() {"w-56 h-full p-4 gap-4 bg-[#663399] text-[#ffffff] translate-x-0 transition-transform duration-500 ease-out"} else {"w-56 h-full p-4 gap-4 bg-[#663399] text-[#ffffff] -translate-x-56 transition-transform duration-500 ease-out"}}>
                <span className="text-xl">"Your workspace"</span>
                <span>"Projects"</span>
                <span>"Activity"</span>
                <button className="bg-[#ffffff] text-[#663399]" onClick={move |_| open.set(false)}>"Close menu"</button>
            </div>
        </div>
    </view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
