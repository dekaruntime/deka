//! Adapter from VM-owned component instances to the existing retained Rust UI tree.
use crate::*;
use deka_native_ui::{Application, Node};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WireNode {
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub classes: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub handler: Option<usize>,
    #[serde(default)]
    pub children: Vec<WireNode>,
}
impl WireNode {
    pub fn into_node(self, id: String) -> Result<Node> {
        let mut style = if self.text.is_some() {
            deka_native_ui::Style::default()
        } else {
            deka_native_ir::element_style(&self.tag)?
        };
        deka_native_ir::apply_classes(&mut style, &self.classes)?;
        let children = self
            .children
            .into_iter()
            .enumerate()
            .map(|(i, c)| c.into_node(format!("{id}/{i}")))
            .collect::<Result<_>>()?;
        Ok(Node {
            id,
            style,
            text: self.text,
            on_click: self.handler,
            children,
        })
    }
}
/// Shared event-to-tree adapter for both desktop windows and the WASM canvas.
/// VM execution happens on load/events; presentation frames only read this tree.
pub struct UiSession {
    component: component::Component,
    tree: Node,
}
impl UiSession {
    pub fn new(program: Program) -> Result<Self> {
        let mut component = component::Component::new(program, Hosts::default())?;
        let frame = component.render()?;
        if !frame.inputs.is_empty() {
            return Err("inputs require a platform text editor host".into());
        }
        Ok(Self {
            component,
            tree: frame.root,
        })
    }
    pub fn tree(&self) -> &Node {
        &self.tree
    }
    pub fn stats(&self) -> HeapStats {
        self.component.stats()
    }
    pub fn instructions(&self) -> u64 {
        self.component.instructions()
    }
    pub fn evaluations(&self) -> usize {
        self.component.evaluations()
    }
    pub fn click(&mut self, handler: usize) -> Result<()> {
        self.component.event(handler, vec![])?;
        let frame = self.component.render()?;
        if !frame.inputs.is_empty() {
            return Err("inputs require a platform text editor host".into());
        }
        reconcile(&mut self.tree, frame.root);
        Ok(())
    }
}
// Preserve existing node allocations when positional structure is unchanged.
// Stable keyed reconciliation is a separate feature, not implied by this adapter.
fn reconcile(current: &mut Node, next: Node) {
    current.style = next.style;
    current.text = next.text;
    current.on_click = next.on_click;
    if current.children.len() == next.children.len() {
        for (current, next) in current.children.iter_mut().zip(next.children) {
            reconcile(current, next);
        }
    } else {
        current.children = next.children;
    }
}
pub struct VmApp {
    session: RefCell<UiSession>,
    error: RefCell<Option<String>>,
}
impl VmApp {
    pub fn instructions(&self) -> u64 {
        self.session.borrow().instructions()
    }
    pub fn new(program: Program) -> Result<Self> {
        Ok(Self {
            session: RefCell::new(UiSession::new(program)?),
            error: RefCell::new(None),
        })
    }
}
impl Application for VmApp {
    fn initial_state(&self) -> Vec<f64> {
        vec![]
    }
    fn render(&self, _: &[f64]) -> Node {
        if let Some(error) = self.error.borrow().as_ref() {
            return Node {
                id: "error".into(),
                style: Default::default(),
                text: Some(format!("VM error: {error}")),
                on_click: None,
                children: vec![],
            };
        }
        self.session.borrow().tree.clone()
    }
    fn event(&self, handler: usize, _: &mut [f64]) {
        if let Err(error) = self.session.borrow_mut().click(handler) {
            *self.error.borrow_mut() = Some(error);
        }
    }
}
/// Produce an identical scene snapshot through either backend without opening a window.
pub fn snapshot<A: Application>(app: A, clicks: usize) -> Result<String> {
    let mut host = deka_native_ui::Host::new(app);
    for _ in 0..clicks {
        host.click(0);
    }
    let renderer = deka_native_ui::scene::Renderer::new();
    let scene = renderer.render_at(&host.render(), 560., 300., 1., 0., true);
    serde_json::to_string(&scene).map_err(|e| e.to_string())
}
