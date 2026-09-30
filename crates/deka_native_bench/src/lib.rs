//! Isolated engine comparison. This is not a production runtime adapter.
use deka_native_ui::{Application, Node, Style};
use std::cell::RefCell;

pub trait Backend: 'static {
    fn increment(&mut self, value: f64) -> f64;
}

pub struct RustBackend;
impl Backend for RustBackend {
    fn increment(&mut self, value: f64) -> f64 {
        value + 1.
    }
}

#[cfg(feature = "quickjs-backend")]
pub mod quickjs;
#[cfg(feature = "v8-backend")]
pub mod v8;

pub struct App<B: Backend> {
    backend: RefCell<B>,
}
impl<B: Backend> App<B> {
    pub fn new(mut backend: B) -> Self {
        assert_eq!(backend.increment(41.), 42.); // Initialize and actually execute the backend before measuring idle.
        Self {
            backend: RefCell::new(backend),
        }
    }
}
impl<B: Backend> Application for App<B> {
    fn initial_state(&self) -> Vec<f64> {
        vec![0.]
    }
    fn render(&self, state: &[f64]) -> Node {
        Node {
            id: "root".into(),
            style: Style {
                padding: 24.,
                gap: 16.,
                background: Some(0xf3efe3),
                ..Default::default()
            },
            text: None,
            on_click: None,
            children: vec![
                Node {
                    id: "title".into(),
                    style: Style {
                        font_size: Some(24.),
                        ..Default::default()
                    },
                    text: Some("Deka native backend comparison".into()),
                    on_click: None,
                    children: vec![],
                },
                Node {
                    id: "counter".into(),
                    style: Style {
                        padding: 16.,
                        radius: 8.,
                        color: Some(0xffffff),
                        background: Some(0x0c8b43),
                        ..Default::default()
                    },
                    text: Some(format!("Count: {}", state[0])),
                    on_click: Some(0),
                    children: vec![],
                },
            ],
        }
    }
    fn event(&self, handler: usize, state: &mut [f64]) {
        if handler == 0 {
            state[0] = self.backend.borrow_mut().increment(state[0]);
        }
    }
}
