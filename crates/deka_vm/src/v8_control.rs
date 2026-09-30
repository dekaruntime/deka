//! Minimal matched control: real V8 state/handlers with the same renderer and wire tree.
//! JS fixture mirrors the DSX demo; this is not the full Deka/React host.
use crate::{Result, ui::WireNode};
use deka_native_ui::{Application, Node};
use std::cell::RefCell;
pub struct V8App {
    js: RefCell<deno_core::JsRuntime>,
    tree: RefCell<Node>,
}
impl V8App {
    pub fn new() -> Result<Self> {
        let mut js = deno_core::JsRuntime::new(Default::default());
        js.execute_script("counter.js", include_str!("../examples/counter-control.js"))
            .map_err(|e| e.to_string())?;
        let tree = read_tree(&mut js)?;
        Ok(Self {
            js: RefCell::new(js),
            tree: RefCell::new(tree),
        })
    }
}
fn read_tree(js: &mut deno_core::JsRuntime) -> Result<Node> {
    let result = js
        .execute_script("snapshot.js", "JSON.stringify(snapshot())")
        .map_err(|e| e.to_string())?;
    deno_core::scope!(scope, js);
    let value = deno_core::v8::Local::new(scope, result);
    let json = value.to_rust_string_lossy(scope);
    let wire: WireNode = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    wire.into_node("root".into())
}
impl Application for V8App {
    fn initial_state(&self) -> Vec<f64> {
        vec![]
    }
    fn render(&self, _: &[f64]) -> Node {
        self.tree.borrow().clone()
    }
    fn event(&self, handler: usize, _: &mut [f64]) {
        let mut js = self.js.borrow_mut();
        let result = js
            .execute_script("event.js", format!("click({handler})"))
            .map_err(|e| e.to_string())
            .and_then(|_| read_tree(&mut js));
        match result {
            Ok(tree) => *self.tree.borrow_mut() = tree,
            Err(e) => eprintln!("V8 control error: {e}"),
        }
    }
}
