//! `cargo run --release -p deka-ui --example text_input`.
use deka_ui::prelude::*;
#[component]
fn App() -> View {
    let text = signal("Type here".to_owned());
    view! {<view className="p-4 gap-4">
        <input value={text} placeholder="Name" className="w-80 h-10"/>
        <textarea value={text} className="w-80 h-24"/>
        <p>"Value: {text}"</p>
    </view>}
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--snapshot") {
        capture(
            args.get(2)
                .expect("--snapshot requires an output directory"),
        );
    } else {
        launch(App)
    }
}
fn capture(path: &str) {
    use deka_native_ui::window::{DesktopSession, KeyInput};
    use winit::event::{Ime, WindowEvent};
    let path = std::path::Path::new(path);
    std::fs::create_dir_all(path).unwrap();
    let mut app = DesktopSession::new(UiApp::new(App));
    app.frame(560., 320., 2.);
    let key = |name: &str| KeyInput {
        name: name.into(),
        down: true,
        ..Default::default()
    };
    app.keyboard(key("tab"));
    app.keyboard(KeyInput {
        command: true,
        ..key("a")
    });
    app.event(&WindowEvent::Ime(Ime::Commit("Hello ".into())), 2.);
    app.event(
        &WindowEvent::Ime(Ime::Preedit("にほんご".into(), Some((12, 12)))),
        2.,
    );
    let save = |app: &mut DesktopSession<UiApp>, name: &str| {
        let scene = app.frame(560., 320., 2.);
        let shot = deka_native_ui::snapshot(scene, 2.).unwrap();
        let mut ppm = format!("P6\n{} {}\n255\n", shot.width, shot.height).into_bytes();
        ppm.extend(
            shot.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| p[..3].iter().copied()),
        );
        std::fs::write(path.join(format!("{name}.ppm")), ppm).unwrap();
    };
    save(&mut app, "ime-preedit");
    app.event(&WindowEvent::Ime(Ime::Commit("日本語".into())), 2.);
    save(&mut app, "ime-commit");
    app.keyboard(KeyInput {
        shift: true,
        ..key("left")
    });
    save(&mut app, "selection");
}
