//! Tests the real native store and handler; all OS windows stay invisible.
use super::super::{Content, GpuState, input::EventLayer, schedule::Schedule, ui::UiContent};
use super::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
struct App {
    pending: Rc<Cell<bool>>,
    turns: Rc<Cell<usize>>,
    drops: Rc<Cell<usize>>,
}
impl Drop for App {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
impl Application for App {
    fn initial_state(&self) -> Vec<f64> {
        vec![]
    }
    fn render(&self, _: &[f64]) -> crate::Node {
        crate::Node {
            id: "root".into(),
            style: Default::default(),
            text: Some("Native".into()),
            on_click: None,
            children: vec![],
        }
    }
    fn event(&self, _: usize, _: &mut [f64]) {}
    fn has_ready_work(&self) -> bool {
        self.pending.get()
    }
    fn run_turn(&mut self, _: usize) -> bool {
        self.pending.set(false);
        self.turns.set(self.turns.get() + 1);
        true
    }
}
#[derive(Default)]
struct Controller {
    requests: Vec<WindowRequest<App>>,
    closed: Rc<RefCell<Vec<WindowToken>>>,
    errors: Rc<RefCell<Vec<String>>>,
}
impl WindowController for Controller {
    type App = App;
    fn requests(&mut self) -> Vec<WindowRequest<App>> {
        std::mem::take(&mut self.requests)
    }
    fn has_requests(&self) -> bool {
        !self.requests.is_empty()
    }
    fn closed(&mut self, id: WindowToken) {
        self.closed.borrow_mut().push(id);
    }
    fn report_error(&self, _: &str, message: String) {
        self.errors.borrow_mut().push(message);
    }
}
fn shell(
    event_loop: &ActiveEventLoop,
    proxy: EventLoopProxy<Wake>,
    app: App,
) -> Shell<UiContent<App>> {
    let window = Arc::new(
        event_loop
            .create_window(Window::default_attributes().with_visible(false))
            .unwrap(),
    );
    let mut content = UiContent::new(app, true);
    content.frame(200., 100., 1.);
    Shell {
        standalone: false,
        close_requested: false,
        events: EventLayer::default(),
        adapter: None,
        proxy,
        content,
        options: Options::new("Native routing", 200., 100.),
        gpu: GpuState::Failed,
        window: Some(window),
        surface: None,
        schedule: Schedule::new(),
        focused: true,
        presented: 0,
        drawn_scale: None,
        shown_with_frame: false,
        menu_installed: false,
        failed: None,
    }
}
pub(crate) fn run(event_loop: &ActiveEventLoop, proxy: EventLoopProxy<Wake>) {
    let closed = Rc::new(RefCell::new(vec![]));
    let errors = Rc::new(RefCell::new(vec![]));
    let drops = Rc::new(Cell::new(0));
    let pending = [
        Rc::new(Cell::new(false)),
        Rc::new(Cell::new(false)),
        Rc::new(Cell::new(false)),
    ];
    let turns = [
        Rc::new(Cell::new(0)),
        Rc::new(Cell::new(0)),
        Rc::new(Cell::new(0)),
    ];
    let ids = [
        WindowToken::allocate(),
        WindowToken::allocate(),
        WindowToken::allocate(),
    ];
    let mut app = Multiple {
        controller: Controller {
            requests: vec![],
            closed: closed.clone(),
            errors: errors.clone(),
        },
        windows: BTreeMap::new(),
        routes: Routes::default(),
        proxy: proxy.clone(),
        reduced: true,
        clipboard: Default::default(),
        services: super::super::services::Services::new(false),
    };
    let mut os = vec![];
    for i in 0..3 {
        let shell = shell(
            event_loop,
            proxy.clone(),
            App {
                pending: pending[i].clone(),
                turns: turns[i].clone(),
                drops: drops.clone(),
            },
        );
        let window = shell.window.as_ref().unwrap().id();
        os.push(window);
        app.routes.register(window, ids[i]);
        app.windows.insert(ids[i], shell);
    }
    let failed = WindowToken::allocate();
    let mut invalid = Options::new("Invalid", 100., 100.);
    invalid.width = f64::NAN;
    app.controller.requests.push(WindowRequest::Open {
        id: failed,
        app: App {
            pending: Rc::new(Cell::new(false)),
            turns: Rc::new(Cell::new(0)),
            drops: drops.clone(),
        },
        options: invalid,
    });
    app.requests(event_loop);
    assert_eq!(
        errors.borrow().len(),
        1,
        "native open failure must reach the controller error sink"
    );
    assert!(errors.borrow()[0].contains("finite and positive"));
    assert_eq!(
        app.windows.len(),
        3,
        "one failed open must preserve existing windows"
    );
    assert_eq!(
        drops.get(),
        1,
        "failed window application must be destructed"
    );
    pending[1].set(true);
    app.user_event(event_loop, Wake::Work);
    assert_eq!(
        [turns[0].get(), turns[1].get(), turns[2].get()],
        [0, 1, 0],
        "only pending native windows consume work wakes"
    );
    for shell in app.windows.values_mut() {
        shell.schedule.presented(false);
    }
    app.user_event(
        event_loop,
        Wake::Accessibility(accesskit_winit::Event {
            window_id: os[1],
            window_event: accesskit_winit::WindowEvent::InitialTreeRequested,
        }),
    );
    assert!(!app.windows[&ids[0]].schedule.wants_frame());
    assert!(app.windows[&ids[1]].schedule.wants_frame());
    assert!(!app.windows[&ids[2]].schedule.wants_frame());
    service_events(&mut app, event_loop, ids[1]);
    // Frame-limited shells ask the native store to dispose them after the batch.
    app.windows.get_mut(&ids[0]).unwrap().options.frames = Some(1);
    app.windows
        .get_mut(&ids[0])
        .unwrap()
        .presented_frame(event_loop, false);
    app.requests(event_loop);
    assert_eq!(app.windows.len(), 2);
    assert!(app.routes.target(os[0]).is_none());
    assert_eq!(
        drops.get(),
        2,
        "close_requested must dispose real native content"
    );
    app.window_event(event_loop, os[1], WindowEvent::CloseRequested);
    assert_eq!(app.windows.len(), 1);
    assert!(app.routes.target(os[1]).is_none());
    assert_eq!(drops.get(), 3);
    app.user_event(
        event_loop,
        Wake::Accessibility(accesskit_winit::Event {
            window_id: os[1],
            window_event: accesskit_winit::WindowEvent::InitialTreeRequested,
        }),
    );
    app.exiting(event_loop);
    assert!(app.windows.is_empty());
    assert!(app.routes.0.is_empty());
    assert_eq!(drops.get(), 4, "exiting drops the final native mount");
    for id in ids {
        assert_eq!(
            closed
                .borrow()
                .iter()
                .filter(|closed| **closed == id)
                .count(),
            1
        );
    }
    println!(
        "native Multiple/NativeStore close, exit, failed-open, work and accessibility routing passed"
    );
}

struct Dialogs {
    calls: Rc<Cell<usize>>,
}
impl super::super::FileDialogs for Dialogs {
    fn choose(
        &mut self,
        kind: super::super::DialogKind,
        options: &super::super::FileDialogOptions,
        parent: Option<&Window>,
    ) -> super::super::DialogResult {
        assert!(parent.is_some(), "NativeStore supplies the real OS parent");
        assert_eq!(options.filters, vec![("Text".into(), vec!["txt".into()])]);
        let i = self.calls.get();
        self.calls.set(i + 1);
        match i {
            0 => {
                assert_eq!(kind, super::super::DialogKind::Open);
                Ok(Some("/selected.txt".into()))
            }
            1 => {
                assert_eq!(kind, super::super::DialogKind::Save);
                Ok(None)
            }
            _ => Err("native provider failure".into()),
        }
    }
}
fn service_events(
    app: &mut Multiple<Controller>,
    event_loop: &ActiveEventLoop,
    window: WindowToken,
) {
    use super::super::{DialogKind, FileDialogOptions, ServiceRequest};
    let calls = Rc::new(Cell::new(0));
    let results = Rc::new(RefCell::new(vec![]));
    app.services.dialogs(Dialogs {
        calls: calls.clone(),
    });
    for kind in [DialogKind::Open, DialogKind::Save, DialogKind::Open] {
        let output = results.clone();
        app.controller
            .requests
            .push(WindowRequest::Service(ServiceRequest::FileDialog {
                window,
                kind,
                options: FileDialogOptions::new().filter("Text", &["txt"]),
                complete: Box::new(move |result| output.borrow_mut().push(result)),
            }));
        app.user_event(event_loop, Wake::Work);
    }
    assert_eq!(calls.get(), 3);
    assert_eq!(
        &*results.borrow(),
        &[
            Ok(Some("/selected.txt".into())),
            Ok(None),
            Err("native provider failure".into())
        ]
    );
    assert_eq!(
        app.controller.errors.borrow().len(),
        2,
        "backend failure reaches callback and app sink; cancellation adds no error"
    );
    assert_eq!(app.controller.errors.borrow()[1], "native provider failure");
    // The real SystemFileDialogs guard returns errors through NativeStore,
    // without showing an interactive panel on the user's screen.
    app.services.dialogs(super::super::SystemFileDialogs);
    let output = results.clone();
    app.controller
        .requests
        .push(WindowRequest::Service(ServiceRequest::FileDialog {
            window,
            kind: DialogKind::Open,
            options: FileDialogOptions::new().filter("Empty", &[]),
            complete: Box::new(move |result| output.borrow_mut().push(result)),
        }));
    app.user_event(event_loop, Wake::Work);
    assert_eq!(app.controller.errors.borrow().len(), 3);
    assert_eq!(
        results.borrow().last().unwrap().as_ref().unwrap_err(),
        "file filters require nonempty extensions"
    );
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        use super::super::{MenuEntry, MenuEvent, MenuItemSpec, MenuSpec};
        let clicks = Rc::new(Cell::new(0));
        let count = clicks.clone();
        let item = MenuItemSpec::new("Native action", move || {
            count.set(count.get() + 1);
            true
        });
        let id = item.id.clone();
        let count = clicks.clone();
        let mut disabled = MenuItemSpec::new("Disabled", move || {
            count.set(count.get() + 100);
            true
        });
        disabled.enabled = false;
        let disabled_id = disabled.id.clone();
        let spec = MenuSpec {
            entries: vec![MenuEntry::Item(item), MenuEntry::Item(disabled)],
        };
        let native = spec.native_menu().expect("build real OS menu objects");
        assert_eq!(native.items().len(), 2);
        app.controller
            .requests
            .push(WindowRequest::Service(ServiceRequest::AppMenu(spec)));
        app.user_event(event_loop, Wake::Work);
        app.user_event(event_loop, Wake::Menu(MenuEvent { id }));
        assert_eq!(
            clicks.get(),
            1,
            "menu OS payload reaches the real Multiple handler"
        );
        app.user_event(event_loop, Wake::Menu(MenuEvent { id: disabled_id }));
        assert_eq!(clicks.get(), 1);
        // A native menu construction failure must be visible through app errors.
        let bad = MenuSpec {
            entries: vec![MenuEntry::Item(MenuItemSpec::new("bad\0label", || true))],
        };
        app.services = super::super::services::Services::new(true);
        app.controller
            .requests
            .push(WindowRequest::Service(ServiceRequest::AppMenu(bad)));
        app.user_event(event_loop, Wake::Work);
        assert!(
            app.controller
                .errors
                .borrow()
                .last()
                .unwrap()
                .contains("NUL")
        );
        app.services = super::super::services::Services::new(false);
    }
    println!("native menu/dialog payloads, parent routing, cancellation and errors passed");
}
