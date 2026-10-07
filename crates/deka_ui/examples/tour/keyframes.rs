use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view className="w-full h-full p-4 gap-4 bg-[#F3EFE3]">
        <button className="w-48 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle keyframes"</button>
        <span>"Custom keyframes, repeated and alternated. Toggle to stop."</span>
        <div className="w-full h-64 p-8 gap-3 overflow-hidden">
            <div className={move || if open.get() {"w-20 h-20 rounded-lg bg-[#0C8B43] frames-x-[0:0,25:80,75:160,100:0] frames-scale-[0:1,50:1.3,100:1] duration-1400 repeat-infinite alternate"} else {"w-20 h-20 rounded-lg bg-[#0C8B43] animate-none"}}/>
        </div>
    </view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
