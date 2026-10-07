use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view className="w-full h-full p-4 gap-4 bg-[#F3EFE3]">
        <button className="w-48 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle spring motion"</button>
        <span>"Click again while it moves to reverse the spring."</span>
        <div className="w-full h-64 p-8 gap-3 overflow-hidden">
            <div className={move || if open.get() {"w-20 h-20 rounded-lg bg-[#663399] translate-x-48 transition-transform spring spring-damping-9"} else {"w-20 h-20 rounded-lg bg-[#663399] translate-x-0 transition-transform spring spring-damping-9"}}/>
        </div>
    </view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
