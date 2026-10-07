//! Native menus and filtered dialogs from Rust handlers. Normal launch is visible.
use deka_ui::prelude::*;
fn app() -> DesktopApp {
    DesktopApp::new(|windows| {
        let selected = signal("No file selected".to_owned());
        let mut count = signal(0);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        windows
            .app_menu(
                Menu::new().standard_app("deka").submenu(
                    "Actions",
                    Menu::new().item(
                        MenuItem::new("Add", move || count += 1)
                            .accelerator("CmdOrCtrl+I")
                            .unwrap(),
                    ),
                ),
            )
            .unwrap();
        windows.open(WindowOptions::new("Native services",520.,340.),move|window| {
            let open=window.clone();let save=window.clone();
            let view=view!{<view className="p-4 gap-4"><p>"Actions: {count}"</p><p>{selected}</p>
                <button onClick={move |_| {open.open_file(FileDialogOptions::new().filter("Text",&["txt","md"]),move|result|if let Ok(Some(path))=result {selected.set(path.display().to_string());}).unwrap();}}>"Open file"</button>
                <button onClick={move |_| {save.save_file(FileDialogOptions::new().file_name("draft.txt").filter("Text",&["txt","md"]),move|result|if let Ok(Some(path))=result {selected.set(path.display().to_string());}).unwrap();}}>"Save file"</button>
            </view>};
            #[cfg(any(target_os="macos",target_os="windows"))]
            let view=view.on(EventKind::ContextMenu,move|event|if let Event::ContextMenu{x,y}=event {
                window.context_menu(Menu::new().item(MenuItem::new("Add",move||count+=1)),x,y).unwrap();
            });
            view
        }).unwrap();
    })
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    #[cfg(target_os = "macos")]
    if args.get(1).is_some_and(|arg| arg == "--menu-smoke") {
        smoke::menu();
        return;
    }
    #[cfg(target_os = "macos")]
    if args.get(1).is_some_and(|arg| arg == "--dialog-smoke") {
        smoke::dialogs(args.get(2).expect("capture directory"));
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--snapshot") {
        use deka_native_ui::window::{
            DialogKind, FileDialogs, MultipleDesktopSession, NativeWindow,
        };
        struct Dialog;
        impl FileDialogs for Dialog {
            fn choose(
                &mut self,
                _: DialogKind,
                _: &FileDialogOptions,
                _: Option<&NativeWindow>,
            ) -> DialogResult {
                Ok(Some("/documents/example.md".into()))
            }
        }
        let mut session = MultipleDesktopSession::new(app());
        session.dialogs(Dialog);
        let (id, os) = session.windows()[0];
        let rect = session.frame(id, 2.).unwrap().targets[0].rect;
        use winit::{
            dpi::PhysicalPosition,
            event::{DeviceId, ElementState, MouseButton, WindowEvent},
        };
        session.event(
            os,
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
            os,
            &WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            2.,
        );
        let shot = deka_native_ui::snapshot(session.frame(id, 2.).unwrap(), 2.).unwrap();
        let mut ppm = format!("P6\n{} {}\n255\n", shot.width, shot.height).into_bytes();
        ppm.extend(
            shot.rgba
                .chunks_exact(4)
                .flat_map(|p| p[..3].iter().copied()),
        );
        std::fs::write(args.get(2).expect("capture file"), ppm).unwrap();
    } else {
        launch(app());
    }
}
#[cfg(target_os = "macos")]
mod smoke {
    use super::*;
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::{NSPoint, NSString};
    use std::path::PathBuf;
    // These native smoke modes run on main. Menu smoke creates no window;
    // dialog smoke creates only its small automatically closed panel owner.
    pub fn menu() {
        use deka_native_ui::window::{MenuEvent, MultipleDesktopSession, NativeContextMenu};
        let app = DesktopApp::new(move |windows| {
            let mut count = signal(0);
            let menu = Menu::new().standard_app("deka").submenu(
                "Actions",
                Menu::new()
                    .item(
                        MenuItem::new("Add", move || count += 1)
                            .accelerator("CmdOrCtrl+I")
                            .unwrap(),
                    )
                    .item(MenuItem::new("Disabled", move || count += 100).enabled(false)),
            );
            windows.app_menu(menu).unwrap();
            windows
                .open(
                    WindowOptions::new("Headless", 320., 160.),
                    move |_| view! {<p>"Count: {count}"</p>},
                )
                .unwrap();
        });
        // SAFETY: Cocoa initialization on main, without activation or a window.
        unsafe {
            let _: *mut AnyObject =
                msg_send![AnyClass::get(c"NSApplication").unwrap(), sharedApplication];
        }
        let mut session = MultipleDesktopSession::with_native_services(app);
        let native = session
            .native_menu()
            .expect("production service installs native menu");
        let actions = native.items()[1].as_submenu_unchecked().clone();
        assert_eq!(actions.items()[0].as_menuitem_unchecked().text(), "Add");
        assert!(!actions.items()[1].as_menuitem_unchecked().is_enabled());
        // SAFETY: the NSMenu belongs to the live muda submenu on the main thread;
        // this invokes Cocoa's actual target/action path, not a Rust handler.
        unsafe {
            let menu = actions.ns_menu().cast::<AnyObject>();
            let _: () = msg_send![menu,performActionForItemAtIndex:0isize];
        }
        let event = MenuEvent::receiver()
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("Cocoa menu action must emit an OS event");
        let handled = session.menu_event(event);
        let id = session.windows()[0].0;
        assert!(
            session
                .frame(id, 1.)
                .unwrap()
                .nodes
                .iter()
                .any(|node| node.text.as_deref() == Some("Count: 1"))
        );
        assert!(handled);
        println!(
            "PASS: native NSMenu target/action -> muda event -> production menu dispatch -> rendered signal"
        );
    }
    struct PanelTask {
        message: String,
        capture: PathBuf,
        attempts: u8,
    }
    fn schedule(task: PanelTask) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(700));
            dispatch2::DispatchQueue::main().exec_async(move || panel(task));
        });
    }
    fn panel(mut task: PanelTask) {
        // SAFETY: AppKit messages run on main. modalWindow belongs only to this
        // process; a unique panel message is checked before capture/accept/cancel.
        unsafe {
            let app: *mut AnyObject =
                msg_send![AnyClass::get(c"NSApplication").unwrap(), sharedApplication];
            let panel: *mut AnyObject = msg_send![app, modalWindow];
            let owned = if panel.is_null() {
                false
            } else {
                let is_panel: objc2::runtime::Bool =
                    msg_send![panel,isKindOfClass:AnyClass::get(c"NSSavePanel").unwrap()];
                if !is_panel.as_bool() {
                    false
                } else {
                    let message: *mut NSString = msg_send![panel, message];
                    message
                        .as_ref()
                        .is_some_and(|message| message.to_string() == task.message)
                }
            };
            if !owned {
                task.attempts += 1;
                if task.attempts > 15 {
                    eprintln!("native smoke panel did not appear");
                    std::process::exit(2);
                }
                schedule(task);
                return;
            }
            eprintln!("cancelling owned panel: {}", task.message);
            let number: isize = msg_send![panel, windowNumber];
            let capture = std::process::Command::new("/usr/sbin/screencapture")
                .args(["-x", "-l", &number.to_string()])
                .arg(&task.capture)
                .status()
                .unwrap();
            if !capture.success() {
                eprintln!("own-panel screenshot unavailable; manual capture required");
            }
            let _: () = msg_send![panel,cancel:std::ptr::null::<AnyObject>()];
        }
    }
    struct DialogSmoke {
        path: PathBuf,
        done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        points: std::cell::Cell<[(f32, f32); 2]>,
    }
    thread_local! {static DRIVER: std::cell::RefCell<Option<std::rc::Rc<DialogSmoke>>> = const {std::cell::RefCell::new(None)};}
    fn click_later(index: usize) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            dispatch2::DispatchQueue::main().exec_async(move || {
                DRIVER.with(|slot| {
                    slot.borrow().as_ref().unwrap().click(index);
                })
            });
        });
    }
    impl DialogSmoke {
        fn app(self: &std::rc::Rc<Self>, live: bool) -> DesktopApp {
            let driver = self.clone();
            DesktopApp::new(move |windows| {
                let status = signal("Ready".to_owned());
                let mut options = WindowOptions::new("deka dialog smoke owner", 420., 220.);
                if live {
                    options.on_frame = Some(Box::new(|frame| {
                        if frame.index == 1 {
                            click_later(0)
                        }
                    }));
                }
                windows.open(options,move|window| {
                    let open=window.clone();let save=window.clone();
                    let opener=driver.clone();let saver=driver.clone();
                    view!{<view className="p-4 gap-4"><p>{status}</p>
                        <button onClick={move |_| {
                            schedule(PanelTask{message:"deka smoke open".into(),capture:opener.path.join("open.png"),attempts:0});
                            let done=opener.done.clone();
                            open.open_file(FileDialogOptions::new().title("deka smoke open").directory(opener.path.join("fixtures")).filter("Text",&["txt"]),move|result| {
                                assert_eq!(result.unwrap(),None);status.set("Open cancelled".into());done.fetch_add(1,std::sync::atomic::Ordering::SeqCst);click_later(1);
                            }).unwrap();
                        }}>"Open file"</button>
                        <button onClick={move |_| {
                            schedule(PanelTask{message:"deka smoke save".into(),capture:saver.path.join("save.png"),attempts:0});
                            let done=saver.done.clone();let close=save.clone();
                            save.save_file(FileDialogOptions::new().title("deka smoke save").directory(saver.path.join("fixtures")).file_name("deka-smoke-save.txt").filter("Text",&["txt"]),move|result| {
                                assert_eq!(result.unwrap(),None);status.set("Save cancelled".into());done.fetch_add(1,std::sync::atomic::Ordering::SeqCst);close.close();
                            }).unwrap();
                        }}>"Save file"</button>
                    </view>}
                }).unwrap();
            })
        }
        fn click(&self, index: usize) {
            // SAFETY: own process windows on main; the unique title selects only
            // this dialog owner. Post events to AppKit, allowing the main queue
            // to drain before a handler enters its modal panel loop.
            unsafe {
                let app: *mut AnyObject =
                    msg_send![AnyClass::get(c"NSApplication").unwrap(), sharedApplication];
                let windows: *mut AnyObject = msg_send![app, windows];
                let count: usize = msg_send![windows, count];
                let mut own = std::ptr::null_mut::<AnyObject>();
                for i in 0..count {
                    let window: *mut AnyObject = msg_send![windows,objectAtIndex:i];
                    let title: *mut NSString = msg_send![window, title];
                    if title
                        .as_ref()
                        .is_some_and(|s| s.to_string() == "deka dialog smoke owner")
                    {
                        own = window;
                        break;
                    }
                }
                assert!(!own.is_null(), "dialog owner must exist");
                let number: isize = msg_send![own, windowNumber];
                let view: *mut AnyObject = msg_send![own, contentView];
                let bounds: objc2_foundation::NSRect = msg_send![view, bounds];
                let (x, y) = self.points.get()[index];
                if index == 0 {
                    let result = std::process::Command::new("/usr/sbin/screencapture")
                        .args(["-x", "-l", &number.to_string()])
                        .arg(self.path.join("owner.png"))
                        .status()
                        .unwrap();
                    if !result.success() {
                        eprintln!("own-window screenshot unavailable; manual capture required");
                    }
                }
                for kind in [5usize, 1, 2] {
                    let event: *mut AnyObject = msg_send![AnyClass::get(c"NSEvent").unwrap(),mouseEventWithType:kind,location:NSPoint::new(f64::from(x),bounds.size.height-f64::from(y)),modifierFlags:0usize,timestamp:0f64,windowNumber:number,context:std::ptr::null::<AnyObject>(),eventNumber:1isize,clickCount:1isize,pressure:1f32];
                    assert!(!event.is_null());
                    let _: () = msg_send![app,postEvent:event,atStart:objc2::runtime::Bool::NO];
                }
            }
        }
    }
    pub fn dialogs(path: &str) {
        use deka_native_ui::window::MultipleDesktopSession;
        let path = std::path::Path::new(path)
            .canonicalize()
            .expect("create capture directory first");
        std::fs::create_dir_all(path.join("fixtures")).unwrap();
        std::fs::write(
            path.join("fixtures/example.txt"),
            "deka native file dialog smoke fixture\n",
        )
        .unwrap();
        let driver = std::rc::Rc::new(DialogSmoke {
            path,
            done: Default::default(),
            points: Default::default(),
        });
        let mut headless = MultipleDesktopSession::new(driver.app(false));
        let id = headless.windows()[0].0;
        let scene = headless.frame(id, 1.).unwrap();
        driver.points.set(std::array::from_fn(|i| {
            let r = scene.targets[i].rect;
            (r.x + 4., r.y + 4.)
        }));
        drop(headless);
        DRIVER.with(|slot| slot.replace(Some(driver.clone())));
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(15));
            eprintln!("native dialog smoke timeout");
            std::process::exit(2);
        });
        launch(driver.app(true));
        assert_eq!(driver.done.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(!driver.path.join("fixtures/deka-smoke-save.txt").exists());
        DRIVER.with(|slot| slot.replace(None));
        println!(
            "PASS: native AppKit mouse -> winit -> Rust handler -> filtered owner panels -> cancellation -> originating signal -> queued window close; no file written"
        );
    }
}
