//! Print what deka_native_ui lays out and paints for the A/B UI page.
use deka_native_ui::Host;
fn main() {
    let host = Host::new(deka_ab::common::UiApp);
    let scene = deka_native_ui::scene::Renderer::new().render(&host.render(), 1600., 1000., 2.);
    println!("paints {} images {} targets {}", scene.paint.len(), scene.images.len(), scene.targets.len());
    for n in scene.nodes.iter().filter(|n| n.id.starts_with("button") || n.id == "row0" || n.id == "label0").take(4) {
        println!("node {} rect {:?} clip {:?} text {:?}", n.id, n.rect, n.clip, n.text);
    }
    for p in scene.paint.iter().filter(|p| p.color == 0x1d3557).take(3) {
        println!("button paint rect {:?} clip {:?} opacity {}", p.rect, p.clip, p.opacity);
    }
}
