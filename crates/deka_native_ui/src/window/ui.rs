//! An [`Application`](crate::Application) in a window: hit testing, keyboard
//! focus (Tab / Shift-Tab, Enter / Space) and the background turn.
use super::{Content, input::Input};
use crate::{Application, Host, Waker, scene::Renderer, scene::Scene};
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
            live,
            waker: None,
            wake_pending: Arc::new(AtomicBool::new(false)),
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
        let targets = &self.scene.targets;
        let current = targets
            .iter()
            .position(|t| Some(&t.id) == self.focused.as_ref());
        let next = match (current, backwards) {
            (None, false) => Some(0),
            (None, true) => targets.len().checked_sub(1),
            (Some(i), false) => Some(i + 1),
            (Some(i), true) => i.checked_sub(1),
        };
        self.focused = next.and_then(|i| targets.get(i)).map(|t| t.id.clone());
    }
}

impl<A: Application> Content for UiContent<A> {
    fn frame(&mut self, width: f32, height: f32, scale: f32) -> &Scene {
        let root = self.host.render();
        self.scene = self.renderer.render_at(
            &root,
            width,
            height,
            scale,
            self.clock.elapsed().as_secs_f64() * 1000.,
            self.reduced_motion,
        );
        if let Some(id) = &self.focused {
            self.scene.focus_ring(id);
        }
        &self.scene
    }

    fn input(&mut self, input: Input) -> bool {
        match input {
            Input::Press { x, y } => {
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
            Input::Key { .. } | Input::Focus(_) => false,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, Reload, Style};
    use std::cell::Cell;
    use std::rc::Rc;

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
