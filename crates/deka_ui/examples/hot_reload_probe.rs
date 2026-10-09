//! Headless development-loop probe using the tour's real component and renderer.
#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
#[path = "tour/counter.rs"]
mod counter;
#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
fn probe_status() -> &'static str {
    "compiled status"
}
#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
fn main() {
    use deka_native_ui::Application;
    let mut host = deka_native_ui::Host::new(deka_ui::UiApp::new(counter::App));
    assert!(host.app.live(), "development host was not scheduled");
    let _ = host.render();
    host.click(0);
    host.click(0);
    host.click(0);
    let renderer = deka_native_ui::scene::Renderer::new();
    let mut revision = 0;
    let mut clicked = false;
    println!(
        "READY pid={} count=3 status={}",
        std::process::id(),
        probe_status()
    );
    loop {
        if revision == 0 || host.refresh() || !host.app.take_errors().is_empty() {
            let node = host.app.tree().snapshot();
            let scene = renderer.render(&node, 800., 600., 1.);
            println!("SCENE {}", serde_json::to_string(&scene).unwrap());
            println!("TREE {}", serde_json::to_string(&snapshot(&node)).unwrap());
            revision += 1;
        }
        // Real handlers must remain callable after a hot patch, not only the
        // last numeric display value. The driver requests an additional click.
        if std::env::args()
            .skip(1)
            .any(|arg| arg == "--click-after-patch")
            && revision == 2
            && !clicked
        {
            clicked = true;
            let _ = host.render();
            host.click(0);
            println!(
                "AFTER_CLICK {}",
                serde_json::to_string(&snapshot(&host.app.tree().snapshot())).unwrap()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
#[cfg(not(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32"))))]
fn main() {
    eprintln!("probe requires debug + hot-reload");
}

#[cfg(all(feature = "hot-reload", debug_assertions, not(target_arch = "wasm32")))]
fn snapshot(node: &deka_native_ir::Node) -> serde_json::Value {
    serde_json::json!({"id":node.id,"style":format!("{:?}",node.style),"text":node.text,"children":node.children.iter().map(snapshot).collect::<Vec<_>>()})
}
