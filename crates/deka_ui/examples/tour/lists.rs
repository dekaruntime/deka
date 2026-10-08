use deka_ui::prelude::*;

#[component]
pub fn App() -> View {
    let projects = ["Deka", "Zega", "My next idea"];
    let selected = signal("Choose a project");
    view! {<view class="p-6 gap-3 bg-[#F3EFE3] text-[#1A1611]">
        <p class="text-xl">"{selected}"</p>
        {projects.into_iter().map(move |name| view! {
            <button class={move || if selected.get().unwrap_or_default() == name {"p-3 bg-[#0C8B43] text-[#ffffff]"} else {"p-3 bg-[#E2DCCF]"}}
                onClick={move |_| selected.set(name)}>"{name}"</button>
        })}
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
