//! One app scope, independently owned windows, and queued handler requests.
use deka_ui::prelude::*;
fn app() -> DesktopApp {
    DesktopApp::new(|windows| {
        let mut count = signal(0);
        for title in ["First", "Second"] {
            let opener = windows.clone();
            windows.open(WindowOptions::new(title, 360., 260.), move |window| view! {
                <view className="p-4 gap-4"><p>"{title}: {count}"</p>
                    <button onClick={move |_| count += 1}>"Add"</button>
                    <button onClick={move |_| { opener.open(WindowOptions::new("New window", 360., 260.), move |child| view! {
                        <view className="p-4 gap-4"><p>"New window: {count}"</p><button onClick={move |_| { child.close(); }}>"Close"</button></view>
                    }).unwrap(); }}>"New window"</button>
                    <button onClick={move |_| { window.close(); }}>"Close"</button>
                </view>
            }).unwrap();
        }
    })
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--snapshot") {
        use deka_native_ui::window::MultipleDesktopSession;
        use winit::{
            dpi::PhysicalPosition,
            event::{DeviceId, ElementState, MouseButton, WindowEvent},
        };
        let path = std::path::Path::new(args.get(2).expect("snapshot directory"));
        std::fs::create_dir_all(path).unwrap();
        let mut session = MultipleDesktopSession::new(app());
        let windows = session.windows();
        let rect = session.frame(windows[0].0, 2.).unwrap().targets[0].rect;
        session.event(
            windows[0].1,
            &WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(
                    f64::from(rect.x + 4.) * 2.,
                    f64::from(rect.y + 4.) * 2.,
                ),
            },
            2.,
        );
        session.event(
            windows[0].1,
            &WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            2.,
        );
        for (index, (window, _)) in windows.into_iter().enumerate() {
            let shot = deka_native_ui::snapshot(session.frame(window, 2.).unwrap(), 2.).unwrap();
            let mut ppm = format!("P6\n{} {}\n255\n", shot.width, shot.height).into_bytes();
            ppm.extend(
                shot.rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|p| p[..3].iter().copied()),
            );
            std::fs::write(path.join(format!("window-{index}.ppm")), ppm).unwrap();
        }
    } else {
        launch(app());
    }
}
