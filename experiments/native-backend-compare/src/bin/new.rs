//! deka's window on winit + wgpu + vello_gpu (crates/deka_native_ui as built).
use native_backend_compare::*;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("compare") {
        let dir = std::path::PathBuf::from(arg("--dir").unwrap_or_else(|| "shots".into()));
        compare(&dir);
        zoom(&dir, "text-sizes", (40, 40, 260, 120), 4);
        zoom(&dir, "paints", (40, 40, 300, 140), 4);
        zoom(&dir, "transforms", (90, 80, 200, 120), 4);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("snap") {
        let out = std::path::PathBuf::from(arg("--out").unwrap_or_else(|| "shots".into()));
        for name in SCENES {
            let (scene, scale) = scene(name);
            match deka_native_ui::snapshot(&scene, scale) {
                Ok(shot) => {
                    write_png(&out.join(format!("{name}-new.png")), shot.width, shot.height, &shot.rgba);
                    println!("{name}: {}x{}", shot.width, shot.height);
                }
                Err(e) => println!("{name}: FAILED {e}"),
            }
        }
        return;
    }
    let protocol = Protocol::new("new");
    let display = require_virtual_display();
    let on_frame = move |_: deka_native_ui::window::Frame| protocol.frame();
    match app_name().as_str() {
        "world" => {
            let mut options = deka_native_ui::world::world_options();
            options.position = Some(window_origin(display, options.width, options.height));
            options.on_frame = Some(Box::new(on_frame));
            deka_native_ui::world::run_with(options, false, true);
        }
        "settings" => {
            let mut options = deka_native_ui::window::Options::new("Settings", 960., 640.);
            options.position = Some(window_origin(display, 960., 640.));
            options.on_frame = Some(Box::new(on_frame));
            deka_native_ui::window::run_with(Settings, options, false);
        }
        _ => {
            let mut options = deka_native_ui::window::Options::new("Deka native experiment", 560., 300.);
            options.background = 0xf3efe3;
            options.position = Some(window_origin(display, 560., 300.));
            options.on_frame = Some(Box::new(on_frame));
            deka_native_ui::window::run_with(vm_app(COUNTER, "Counter"), options, false);
        }
    }
}
