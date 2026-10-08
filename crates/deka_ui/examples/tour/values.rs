use deka_ui::prelude::*;

struct Release {
    minor: i32,
    _major: i32,
}

#[component]
pub fn App() -> View {
    let language = "DekaScript";
    let release = Release {
        _major: 0,
        minor: 60,
    };
    let mut clicks = signal(0);
    view! {<view class="p-6 gap-3 bg-[#F3EFE3] text-[#1A1611]">
        <p class="text-2xl">"Hello, {language}"</p>
        <p>"Minor version: "{release.minor}</p>
        <p>"Clicks: {clicks}"</p>
        <button onClick={move |_| clicks += 1}>"Change a binding"</button>
    </view>}
}

#[cfg(all(not(test), feature = "desktop", not(target_arch = "wasm32")))]
#[allow(dead_code)] // This source also supplies App to the shared tour registry.
fn main() {
    deka_ui::launch(App);
}
