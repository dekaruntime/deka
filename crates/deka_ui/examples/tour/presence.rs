use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let open = signal(false);
    view! {<view className="w-full h-full p-4 gap-4 bg-[#F3EFE3]">
        <button className="w-48 bg-[#663399] text-[#ffffff]" onClick={move |_| open.toggle()}>"Toggle enter and exit"</button>
        <span>"Toggle mounts and unmounts the card."</span>
        <div className="w-full h-64 p-8 gap-3 overflow-hidden">
            {move || open.get().then(|| view!{
                <div className="w-64 p-4 gap-3 rounded-lg bg-[#ffffff] enter-scale exit-slide duration-600">
                    <span className="text-xl">"Hello again"</span>
                    <span>"Mounted on entry, visually retained during exit."</span>
                </div>
            })}
        </div>
    </view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
