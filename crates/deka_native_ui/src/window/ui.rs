//! An [`Application`](crate::Application) in a window: hit testing, keyboard
//! focus (Tab / Shift-Tab, Enter / Space) and the background turn.
use super::{
    Content,
    editor::Editor,
    input::{EventLayer, Input, KeyInput},
};
use crate::{Application, Host, Waker, scene::Renderer, scene::Scene};
use std::collections::BTreeMap;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// How often a live application (`deka dev`) is asked for a reload.
const LIVE_POLL: Duration = Duration::from_millis(100);

pub(crate) struct UiContent<A: Application> {
    host: Host<A>,
    renderer: Renderer,
    scene: Scene,
    clock: Instant,
    reduced_motion: bool,
    focused: Option<String>,
    active: bool,
    editors: BTreeMap<String, Editor>,
    dragging: bool,
    clipboard: Box<dyn TextClipboard>,
    live: bool,
    waker: Option<Waker>,
    wake_pending: Arc<AtomicBool>,
}

impl<A: Application> UiContent<A> {
    pub(crate) fn new(app: A, reduced_motion: bool) -> Self {
        let live = app.live();
        Self {
            host: Host::new(app),
            renderer: Renderer::new(),
            scene: Scene::default(),
            clock: Instant::now(),
            reduced_motion,
            focused: None,
            active: true,
            editors: BTreeMap::new(),
            dragging: false,
            clipboard: Box::new(SystemClipboard(None)),
            live,
            waker: None,
            wake_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    fn publish(&mut self, id: &str) {
        let editor = self.editors.get_mut(id).unwrap();
        if editor.text.is_composing() {
            return;
        }
        let value = editor.text.text().to_string();
        if value != editor.published {
            editor.published = value.clone();
            self.host.app.text_input(id, value);
        }
    }
    fn click_focused(&mut self) -> bool {
        let handler = self
            .scene
            .targets
            .iter()
            .find(|t| Some(&t.id) == self.focused.as_ref())
            .map(|t| t.handler);
        if let Some(handler) = handler {
            self.host.click(handler);
        }
        handler.is_some()
    }

    fn cycle_focus(&mut self, backwards: bool) {
        let targets: Vec<_> = self
            .scene
            .nodes
            .iter()
            .filter(|n| {
                self.editors.contains_key(&n.id) || self.scene.targets.iter().any(|t| t.id == n.id)
            })
            .map(|n| n.id.clone())
            .collect();
        let current = targets
            .iter()
            .position(|id| Some(id) == self.focused.as_ref());
        let next = match (current, backwards) {
            (None, false) => Some(0),
            (None, true) => targets.len().checked_sub(1),
            (Some(i), false) => Some(i + 1),
            (Some(i), true) => i.checked_sub(1),
        };
        if let Some(editor) = self
            .focused
            .as_ref()
            .and_then(|id| self.editors.get_mut(id))
        {
            editor.cancel();
        }
        self.focused = next.and_then(|i| targets.get(i)).cloned();
    }
}

impl<A: Application> Content for UiContent<A> {
    fn frame(&mut self, width: f32, height: f32, scale: f32) -> &Scene {
        let root = self.host.render();
        let controls = self.host.app.text_controls();
        self.editors
            .retain(|id, _| controls.iter().any(|c| &c.id == id));
        for c in &controls {
            let editor = self
                .editors
                .entry(c.id.clone())
                .or_insert_with(|| Editor::new(&c.value, c.multiline));
            editor.sync(&c.value, c.controlled);
        }
        if self.focused.as_ref().is_some_and(|id| {
            !self.editors.contains_key(id) && !self.scene.targets.iter().any(|t| &t.id == id)
        }) {
            self.focused = None;
        }
        self.scene = self.renderer.render_at(
            &root,
            width,
            height,
            scale,
            self.clock.elapsed().as_secs_f64() * 1000.,
            self.reduced_motion,
        );
        for c in controls {
            if let Some(node) = self.scene.nodes.iter().find(|n| n.id == c.id) {
                let editor = self.editors.get_mut(&c.id).unwrap();
                editor.place(node.rect, node.clip);
                editor.decoration(
                    &mut self.scene,
                    self.active && self.focused.as_ref() == Some(&c.id),
                );
                let value = if editor.text.raw_text().is_empty() {
                    &c.placeholder
                } else {
                    editor.text.raw_text()
                };
                self.renderer.editor_text(
                    &mut self.scene,
                    value,
                    editor.multiline.then_some((editor.rect.width - 8.).max(1.)),
                    editor.origin,
                    editor.clip,
                    scale,
                );
            }
        }
        if let Some(id) = &self.focused {
            self.scene.focus_ring(id);
        }
        &self.scene
    }

    fn input(&mut self, input: Input) -> bool {
        match input {
            Input::Press { x, y } => {
                if let Some(id) = self
                    .scene
                    .nodes
                    .iter()
                    .rev()
                    .find(|n| {
                        self.editors.contains_key(&n.id)
                            && n.rect.contains(x, y)
                            && n.clip.contains(x, y)
                    })
                    .map(|n| n.id.clone())
                {
                    if let Some(previous) = self
                        .focused
                        .as_ref()
                        .and_then(|id| self.editors.get_mut(id))
                    {
                        previous.cancel();
                    }
                    self.focused = Some(id.clone());
                    self.dragging = true;
                    self.editors.get_mut(&id).unwrap().point(x, y, false);
                    return true;
                }
                let Some(target) = self.scene.hit(x, y) else {
                    return false;
                };
                let (id, handler) = (target.id.clone(), target.handler);
                self.focused = Some(id);
                self.host.click(handler);
                true
            }
            Input::Key {
                name,
                down: true,
                shift,
                ..
            } => match name.as_str() {
                "tab" => {
                    self.cycle_focus(shift);
                    true
                }
                "enter" | "space" => self.click_focused(),
                _ => false,
            },
            Input::EditKey(key) => {
                if key.name == "tab" && key.down {
                    self.cycle_focus(key.shift);
                    return true;
                }
                let Some(id) = self.focused.clone() else {
                    return false;
                };
                let Some(editor) = self.editors.get_mut(&id) else {
                    return self.input(Input::Key {
                        name: key.name,
                        down: key.down,
                        repeat: false,
                        shift: key.shift,
                    });
                };
                let handled = if key.command
                    && key.down
                    && matches!(key.name.as_str(), "c" | "x" | "v")
                    && !editor.text.is_composing()
                {
                    if key.name == "v" {
                        match self.clipboard.get() {
                            Ok(text) if !text.is_empty() => editor.insert(&text),
                            Ok(_) => {}
                            Err(error) => self.host.app.report_error("clipboard", error),
                        }
                    } else if let Some(text) = editor.text.selected_text() {
                        match self.clipboard.set(text) {
                            Ok(()) if key.name == "x" => editor.delete_selected(),
                            Ok(()) => {}
                            Err(error) => self.host.app.report_error("clipboard", error),
                        }
                    }
                    true
                } else {
                    editor.key(&key)
                };
                let composing = editor.text.is_composing();
                self.publish(&id);
                let changed = key.down
                    && !composing
                    && self
                        .host
                        .app
                        .key_input(&id, key_name_for_handler(&key.name));
                handled || changed
            }
            Input::Text(value) => {
                let Some(id) = self.focused.clone() else {
                    return false;
                };
                let Some(editor) = self.editors.get_mut(&id) else {
                    return false;
                };
                editor.insert(&value);
                self.publish(&id);
                true
            }
            Input::Preedit(value, cursor) => {
                let Some(editor) = self
                    .focused
                    .as_ref()
                    .and_then(|id| self.editors.get_mut(id))
                else {
                    return false;
                };
                editor.compose(&value, cursor);
                true
            }
            Input::Move { x, y } => {
                if self.dragging
                    && let Some(editor) = self
                        .focused
                        .as_ref()
                        .and_then(|id| self.editors.get_mut(id))
                {
                    editor.point(x, y, true);
                    return true;
                }
                false
            }
            Input::Release => {
                self.dragging = false;
                false
            }
            Input::Focus(false) => {
                self.active = false;
                self.dragging = false;
                if let Some(editor) = self
                    .focused
                    .as_ref()
                    .and_then(|id| self.editors.get_mut(id))
                {
                    editor.cancel();
                }
                true
            }
            Input::Focus(true) => {
                self.active = true;
                true
            }
            Input::Key { .. } => false,
        }
    }

    fn ime_area(&self) -> Option<crate::scene::Rect> {
        if !self.active {
            return None;
        }
        self.focused
            .as_ref()
            .and_then(|id| self.editors.get(id))
            .map(Editor::ime_area)
    }
    fn turn(&mut self) -> bool {
        self.wake_pending.store(false, Ordering::Release);
        let reloaded = self.host.refresh();
        let changed = self.host.run_turn(4096);
        if self.host.has_ready_work()
            && let Some(waker) = &self.waker
        {
            waker.wake();
        }
        reloaded || changed
    }

    fn turn_interval(&self) -> Option<Duration> {
        self.live.then_some(LIVE_POLL)
    }

    fn set_waker(&mut self, waker: Waker) {
        let pending = self.wake_pending.clone();
        let waker = Waker::new(move || {
            if !pending.swap(true, Ordering::AcqRel) {
                waker.wake();
            }
        });
        self.waker = Some(waker.clone());
        self.host.set_waker(waker);
    }

    #[cfg(test)]
    fn scene(&self) -> &Scene {
        &self.scene
    }
}

/// Text clipboard seam; the desktop uses a long-lived arboard clipboard.
pub trait TextClipboard {
    fn get(&mut self) -> Result<String, String>;
    fn set(&mut self, text: &str) -> Result<(), String>;
}
struct SystemClipboard(Option<arboard::Clipboard>);
impl SystemClipboard {
    fn clipboard(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.0.is_none() {
            self.0 = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        self.0
            .as_mut()
            .ok_or_else(|| "clipboard unavailable".into())
    }
}
impl TextClipboard for SystemClipboard {
    fn get(&mut self) -> Result<String, String> {
        match self.clipboard()?.get_text() {
            Ok(text) => Ok(text),
            Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
            Err(error) => Err(error.to_string()),
        }
    }
    fn set(&mut self, text: &str) -> Result<(), String> {
        self.clipboard()?.set_text(text).map_err(|e| e.to_string())
    }
}
fn key_name_for_handler(name: &str) -> String {
    match name {
        "enter" => "Enter",
        "tab" => "Tab",
        "escape" => "Escape",
        "space" => " ",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        n => n,
    }
    .into()
}
/// A headless desktop event driver. Uses the window's production translation,
/// editing, focus, dispatch, layout and paint path without opening a window.
pub struct DesktopSession<A: Application> {
    content: UiContent<A>,
    events: EventLayer,
}
impl<A: Application> DesktopSession<A> {
    pub fn new(app: A) -> Self {
        Self {
            content: UiContent::new(app, true),
            events: EventLayer::default(),
        }
    }
    pub fn clipboard(&mut self, clipboard: impl TextClipboard + 'static) {
        self.content.clipboard = Box::new(clipboard);
    }
    pub fn frame(&mut self, width: f32, height: f32, scale: f32) -> &Scene {
        self.content.frame(width, height, scale)
    }
    pub fn event(&mut self, event: &winit::event::WindowEvent, scale: f64) -> bool {
        self.events
            .translate(event, scale)
            .is_some_and(|i| self.content.input(i))
    }
    pub fn keyboard(&mut self, key: KeyInput) -> bool {
        self.content.input(Input::EditKey(key))
    }
    pub fn app(&self) -> &A {
        &self.content.host.app
    }
    pub fn focus(&self) -> Option<&str> {
        self.content.focused.as_deref()
    }
    pub fn ime_area(&self) -> Option<crate::scene::Rect> {
        self.content.ime_area()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, Reload, Style};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn system_clipboard_round_trip() {
        let mut clipboard = SystemClipboard(None);
        if let Err(error) = clipboard.clipboard() {
            #[cfg(target_os = "linux")]
            {
                if std::env::var_os("DISPLAY").is_none()
                    && std::env::var_os("WAYLAND_DISPLAY").is_none()
                {
                    use std::io::Write as _;
                    // Direct stderr bypasses libtest's successful-test capture.
                    writeln!(std::io::stderr().lock(),
                        "SKIP system_clipboard_round_trip: headless Linux has no display clipboard: {error}"
                    ).expect("write the CI-visible clipboard skip reason");
                    return;
                }
                panic!("platform clipboard failed on a display host: {error}");
            }
            #[cfg(not(target_os = "linux"))]
            panic!("system clipboard must exist on this desktop host: {error}");
        }
        let previous = clipboard.get().unwrap();
        let text = format!("Deka clipboard round trip 日本 {}", std::process::id());
        let result = clipboard.set(&text).and_then(|()| clipboard.get());
        clipboard.set(&previous).unwrap();
        assert_eq!(result.unwrap(), text);
    }

    /// Two buttons; each click adds its index + 1 to the state.
    struct Buttons {
        reloads: Rc<Cell<u32>>,
    }
    impl Application for Buttons {
        fn initial_state(&self) -> Vec<f64> {
            vec![0.]
        }
        fn render(&self, state: &[f64]) -> Node {
            let button = |i: usize| Node {
                id: format!("b{i}"),
                style: Style {
                    width: crate::Length::Px(100.),
                    height: crate::Length::Px(40.),
                    background: Some(0x336699),
                    ..Default::default()
                },
                text: Some(format!("{}", state[0])),
                on_click: Some(i),
                children: vec![],
            };
            Node {
                id: "root".into(),
                style: Style::default(),
                text: None,
                on_click: None,
                children: vec![button(0), button(1)],
            }
        }
        fn event(&self, handler: usize, state: &mut [f64]) {
            state[0] += handler as f64 + 1.;
        }
        fn poll_reload(&mut self) -> Reload {
            self.reloads.set(self.reloads.get() + 1);
            Reload::Reset
        }
        fn live(&self) -> bool {
            true
        }
    }

    fn content() -> (UiContent<Buttons>, Rc<Cell<u32>>) {
        let reloads = Rc::new(Cell::new(0));
        let mut ui = UiContent::new(
            Buttons {
                reloads: reloads.clone(),
            },
            true,
        );
        ui.frame(400., 300., 2.);
        (ui, reloads)
    }

    fn key(name: &str, shift: bool) -> Input {
        Input::Key {
            name: name.into(),
            down: true,
            repeat: false,
            shift,
        }
    }

    #[test]
    fn a_press_on_a_target_runs_its_handler_and_focuses_it() {
        let (mut ui, _) = content();
        let second = ui.scene().targets[1].rect;
        let (x, y) = (second.x + 5., second.y + 5.);
        assert!(ui.input(Input::Press { x, y }), "a hit redraws");
        assert_eq!(ui.host.state[0], 2.);
        assert_eq!(ui.focused.as_deref(), Some("b1"));
        assert!(
            !ui.input(Input::Press { x: 390., y: 290. }),
            "a miss changes nothing"
        );
        assert_eq!(ui.host.state[0], 2.);
        // The next frame shows the focus ring around the clicked target.
        ui.frame(400., 300., 2.);
        assert!(ui.scene().paint.iter().any(|p| p.color == 0xffc800));
    }

    #[test]
    fn tab_moves_focus_and_enter_or_space_activates_it() {
        let (mut ui, _) = content();
        assert!(ui.input(key("tab", false)));
        assert_eq!(ui.focused.as_deref(), Some("b0"));
        assert!(ui.input(key("tab", false)));
        assert_eq!(ui.focused.as_deref(), Some("b1"));
        assert!(ui.input(key("tab", true)), "shift-tab goes back");
        assert_eq!(ui.focused.as_deref(), Some("b0"));
        assert!(ui.input(key("enter", false)));
        assert_eq!(ui.host.state[0], 1.);
        assert!(ui.input(key("space", false)));
        assert_eq!(ui.host.state[0], 2.);
        assert!(!ui.input(key("x", false)), "other keys are ignored");
        let up = Input::Key {
            name: "enter".into(),
            down: false,
            repeat: false,
            shift: false,
        };
        assert!(!ui.input(up), "releases do not click");
        assert_eq!(ui.host.state[0], 2.);
    }

    #[test]
    fn the_window_waker_reaches_the_application() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        struct Async {
            waker: Arc<Mutex<Option<Waker>>>,
        }
        impl Application for Async {
            fn initial_state(&self) -> Vec<f64> {
                vec![]
            }
            fn render(&self, _: &[f64]) -> Node {
                Node {
                    id: "root".into(),
                    style: Style::default(),
                    text: None,
                    on_click: None,
                    children: vec![],
                }
            }
            fn event(&self, _: usize, _: &mut [f64]) {}
            fn set_waker(&mut self, waker: Waker) {
                *self.waker.lock().unwrap() = Some(waker);
            }
        }
        let slot = Arc::new(Mutex::new(None));
        let mut ui = UiContent::new(
            Async {
                waker: slot.clone(),
            },
            true,
        );
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        ui.set_waker(Waker::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }));
        let waker = slot
            .lock()
            .unwrap()
            .clone()
            .expect("the app received a waker");
        std::thread::spawn(move || waker.wake()).join().unwrap();
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "waking from another thread"
        );
        assert_eq!(
            ui.turn_interval(),
            None,
            "a non-live app is only turned on wake"
        );
    }

    #[test]
    fn the_background_turn_is_the_reload_poll() {
        let (mut ui, reloads) = content();
        assert_eq!(ui.turn_interval(), Some(LIVE_POLL));
        ui.input(key("tab", false));
        ui.input(key("enter", false));
        assert_eq!(ui.host.state[0], 1.);
        assert!(ui.turn(), "a reset reload redraws");
        assert_eq!(reloads.get(), 1);
        assert_eq!(ui.host.state[0], 0., "the reset restored the initial state");
    }
    #[test]
    fn bounded_turns_coalesce_wakes_and_redraw_only_changed_views() {
        use std::sync::atomic::AtomicUsize;
        struct Burst {
            remaining: usize,
            waker: Option<Waker>,
        }
        impl Application for Burst {
            fn initial_state(&self) -> Vec<f64> {
                vec![]
            }
            fn render(&self, _: &[f64]) -> Node {
                Node {
                    id: "root".into(),
                    style: Style::default(),
                    text: Some(
                        if self.remaining == 0 {
                            "done"
                        } else {
                            "waiting"
                        }
                        .into(),
                    ),
                    on_click: None,
                    children: vec![],
                }
            }
            fn event(&self, _: usize, _: &mut [f64]) {}
            fn set_waker(&mut self, waker: Waker) {
                self.waker = Some(waker);
            }
            fn has_ready_work(&self) -> bool {
                self.remaining > 0
            }
            fn run_turn(&mut self, budget: usize) -> bool {
                assert_eq!(budget, 4096);
                if self.remaining == 0 {
                    return false;
                }
                self.remaining -= 1;
                if self.remaining > 0 {
                    self.waker.as_ref().unwrap().wake();
                }
                self.remaining == 0
            }
        }
        let mut ui = UiContent::new(
            Burst {
                remaining: 3,
                waker: None,
            },
            true,
        );
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        ui.set_waker(Waker::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(ui.turn_interval(), None);
        assert!(!ui.turn(), "background progress alone does not redraw");
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            1,
            "VM and window wakes coalesce"
        );
        assert!(!ui.turn());
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        assert!(ui.turn(), "completed work changed the visible text");
        assert_eq!(ui.host.render().text.as_deref(), Some("done"));
        for _ in 0..64 {
            assert!(!ui.turn(), "zero idle redraws");
        }
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            2,
            "idle work does not wake the window"
        );
    }
}
