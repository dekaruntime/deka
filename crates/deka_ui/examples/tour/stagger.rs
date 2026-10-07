use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view className="w-full h-full p-4 gap-4 bg-[#F3EFE3]">
        <button className="w-48 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle staggered entrance"</button>
        <span>"A parent spaces its children's entrances by 120ms."</span>
        <div className="w-full h-64 p-8 gap-3 overflow-hidden">
            {move || open.get().unwrap_or_default().then(|| view!{
                <div className="w-64 gap-3 stagger-120">
                    <div className="p-3 bg-[#663399] text-[#ffffff] enter-slide exit-fade duration-400">"Projects"</div>
                    <div className="p-3 bg-[#663399] text-[#ffffff] enter-slide duration-400">"Activity"</div>
                    <div className="p-3 bg-[#663399] text-[#ffffff] enter-slide duration-400">"Settings"</div>
                </div>
            })}
        </div>
    </view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
