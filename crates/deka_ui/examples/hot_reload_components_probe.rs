//! Two keyed opaque Rust components exercised without a display.
#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
use deka_ui::prelude::*;
#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
#[derive(Clone)]
struct State {
    count: Signal<i32>,
}
#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
#[component]
fn Counter(id: String) -> View {
    let count = signal(0);
    view! {<span id={id}>"Value: {count}"</span>}.with_state(ComponentState::new(State { count }))
}
#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
#[component]
fn App() -> View {
    view! {<view><Counter id="first"/><Counter id="second"/></view>}
}
#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
fn main() {
    let mut app = UiApp::new(App);
    let before = app.tree();
    let first = before.get_element_by_id("first").unwrap();
    let second = before.get_element_by_id("second").unwrap();
    first.component_state::<State>().unwrap().count.set(7);
    second.component_state::<State>().unwrap().count.set(11);
    println!("READY pid={} count=7,11", std::process::id());
    loop {
        if matches!(
            app.poll_hot_reload(),
            deka_ui::hot_reload::ReloadStatus::Patched { .. }
        ) {
            let after = app.tree();
            assert!(after.get_element_by_id("first").unwrap() == first);
            assert!(after.get_element_by_id("second").unwrap() == second);
            assert_eq!(
                first
                    .component_state::<State>()
                    .unwrap()
                    .count
                    .try_get()
                    .unwrap(),
                7
            );
            assert_eq!(
                second
                    .component_state::<State>()
                    .unwrap()
                    .count
                    .try_get()
                    .unwrap(),
                11
            );
            assert_eq!(first.text_content(), "Value: 7");
            assert_eq!(second.text_content(), "Value: 11");
            println!(
                "COMPONENTS {}",
                serde_json::json!({"order":after.query_all("span").unwrap().iter().map(|element|element.get_attribute("id").unwrap()).collect::<Vec<_>>(),"state":[7,11]})
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
#[cfg(not(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32"))))]
fn main() {
    eprintln!("probe requires debug + hot-reload");
}
