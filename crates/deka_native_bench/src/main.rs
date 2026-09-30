//! Controlled size comparison: identical native UI, with a Rust or V8 backend.
use deka_native_ui::{Application, Node, Style};
use std::cell::RefCell;

#[cfg(not(feature = "v8-backend"))]
struct Backend;
#[cfg(not(feature = "v8-backend"))]
impl Backend {
    fn new() -> Self {
        Self
    }
    fn increment(&mut self, value: f64) -> f64 {
        value + 1.
    }
}
#[cfg(feature = "v8-backend")]
struct Backend {
    js: deno_core::JsRuntime,
}
#[cfg(feature = "v8-backend")]
impl Backend {
    fn new() -> Self {
        let mut js = deno_core::JsRuntime::new(Default::default());
        js.execute_script("backend.js", "globalThis.increment = n => n + 1;")
            .expect("initialize JS backend");
        Self { js }
    }
    fn increment(&mut self, value: f64) -> f64 {
        let result = self
            .js
            .execute_script("call.js", format!("increment({value})"))
            .expect("call JS backend");
        deno_core::scope!(scope, self.js);
        let value = deno_core::v8::Local::new(scope, result);
        value.number_value(scope).expect("numeric backend result")
    }
}
struct App {
    backend: RefCell<Backend>,
}
impl App {
    fn new() -> Self {
        let mut backend = Backend::new();
        assert_eq!(backend.increment(41.), 42.); // Initialize and actually execute the backend before measuring idle.
        Self {
            backend: RefCell::new(backend),
        }
    }
}
impl Application for App {
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
fn main() {
    deka_native_ui::run(App::new());
}
