use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let mut count = signal(0);
    let mut other = signal(10);
    view! {<view className="p-4 gap-2 bg-[#F3EFE3] text-[#1A1611]">
        <p className="text-xl">"A view. A binding. No React."</p>
        <p>"State lives in the VM. Rust draws the view."</p>
        <p className="text-xl">"Count: {count}"</p>
        <div className="flex-row gap-3">
            <button onClick={move |_| count += 1}>"Add one"</button>
            <button onClick={move |_| count.set(0)}>"Reset count"</button>
        </div>
        <p>"Independent value: {other}"</p>
        <button onClick={move |_| other += 10}>"Change other"</button>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
