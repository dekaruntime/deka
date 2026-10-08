//! Main-thread winit ingress tests. The windows stay invisible and have no surface.
use super::*;
use crate::{SemanticNode, SemanticRole};
use std::cell::Cell;
use std::rc::Rc;

struct App {
    blocked: Rc<Cell<bool>>,
    clicks: Rc<Cell<usize>>,
}
impl Application for App {
    fn initial_state(&self) -> Vec<f64> {
        vec![]
    }
    fn render(&self, _: &[f64]) -> crate::Node {
        crate::Node {
            id: "parent".into(),
            style: Default::default(),
            text: None,
            on_click: None,
            children: vec![crate::Node {
                id: "button".into(),
                style: Default::default(),
                text: Some("Activate".into()),
                on_click: Some(0),
                children: vec![],
            }],
        }
    }
    fn event(&self, _: usize, _: &mut [f64]) {
        self.clicks.set(self.clicks.get() + 1);
    }
    fn semantics(&self) -> Vec<SemanticNode> {
        let node = |id: &str, parent, role, hidden, disabled, tab_index, clickable| SemanticNode {
            id: id.into(),
            parent,
            role,
            name: id.into(),
            value: String::new(),
            hidden,
            disabled,
            tab_index,
            clickable,
        };
        vec![
            node(
                "parent",
                None,
                SemanticRole::Group,
                false,
                false,
                None,
                false,
            ),
            node(
                "button",
                Some("parent".into()),
                SemanticRole::Button,
                false,
                self.blocked.get(),
                Some(0),
                true,
            ),
        ]
    }
}
struct TestLoop {
    proxy: EventLoopProxy<Wake>,
    ran: Rc<Cell<bool>>,
}
impl ApplicationHandler<Wake> for TestLoop {
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {
        // Tests inject the public winit payloads directly in resumed; unsolicited
        // events for these permanently invisible windows require no rendering.
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.ran.replace(true) {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .expect("invisible native test window"),
        );
        let clicks = Rc::new(Cell::new(0));
        let blocked = Rc::new(Cell::new(false));
        let mut content = ui::UiContent::new(
            App {
                blocked: blocked.clone(),
                clicks: clicks.clone(),
            },
            true,
        );
        content.frame(200., 100., 1.);
        let mut shell = Shell {
            standalone: false,
            close_requested: false,
            proxy: self.proxy.clone(),
            content,
            options: Options::new("Test", 200., 100.),
            gpu: GpuState::Failed,
            window: Some(window.clone()),
            surface: None,
            schedule: Schedule::new(),
            events: EventLayer::default(),
            adapter: Some(accesskit_winit::Adapter::with_event_loop_proxy(
                event_loop,
                &window,
                self.proxy.clone(),
            )),
            focused: true,
            presented: 0,
            drawn_scale: None,
            shown_with_frame: false,
            menu_installed: false,
            failed: None,
        };
        // A presented idle frame may query the adapter, but must not call the
        // projection factory or retain any platform tree without activation.
        let built = Rc::new(Cell::new(false));
        let flag = built.clone();
        shell.adapter.as_mut().unwrap().update_if_active(|| {
            flag.set(true);
            shell.content.accessibility(1.)
        });
        assert!(!built.get(), "inactive OS adapter must not build a tree");
        shell.user_event(
            event_loop,
            Wake::Accessibility(accesskit_winit::Event {
                window_id: window.id(),
                window_event: accesskit_winit::WindowEvent::InitialTreeRequested,
            }),
        );
        let full = shell.content.accessibility(1.);
        assert!(full.tree.is_some());
        let button = full
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("button"))
            .unwrap()
            .0;
        let request = |action| {
            Wake::Accessibility(accesskit_winit::Event {
                window_id: window.id(),
                window_event: accesskit_winit::WindowEvent::ActionRequested(
                    accesskit::ActionRequest {
                        action,
                        target_tree: accesskit::TreeId::ROOT,
                        target_node: button,
                        data: None,
                    },
                ),
            })
        };
        shell.schedule.presented(false);
        shell.user_event(event_loop, request(accesskit::Action::Click));
        assert_eq!(clicks.get(), 1);
        assert!(
            shell.schedule.wants_frame(),
            "OS action invalidates the real shell scheduler"
        );
        shell.user_event(event_loop, request(accesskit::Action::Focus));
        assert_eq!(shell.content.accessibility(1.).focus, button);
        blocked.set(true);
        shell.schedule.presented(false);
        shell.user_event(event_loop, request(accesskit::Action::Click));
        shell.user_event(event_loop, request(accesskit::Action::Focus));
        assert_eq!(clicks.get(), 1);
        assert!(
            !shell.schedule.wants_frame(),
            "disabled action is rejected before repaint"
        );
        shell.user_event(
            event_loop,
            Wake::Accessibility(accesskit_winit::Event {
                window_id: window.id(),
                window_event: accesskit_winit::WindowEvent::AccessibilityDeactivated,
            }),
        );
        assert!(
            shell.content.accessibility(1.).tree.is_some(),
            "deactivation releases the previous projection"
        );
        super::multiple::native_tests::run(event_loop, self.proxy.clone());
        shell.exiting(event_loop);
        event_loop.exit();
    }
}
pub fn run() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("SKIP native_events: headless Linux has no native window system");
        return;
    }
    let mut builder = EventLoop::<Wake>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        builder.with_default_menu(false);
    }
    let event_loop = builder.build().expect("native event loop");
    let ran = Rc::new(Cell::new(false));
    let mut app = TestLoop {
        proxy: event_loop.create_proxy(),
        ran: ran.clone(),
    };
    event_loop.run_app(&mut app).expect("native event routing");
    assert!(ran.get(), "native event checks must execute");
    println!("native Shell::user_event checks passed");
}
