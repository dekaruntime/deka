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
    /// Authored scalar attributes retained independently of renderer snapshots.
    #[serde(default)]
    pub attributes: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub handler: Option<usize>,
    #[serde(default)]
    pub children: Vec<WireNode>,
}
impl WireNode {
    pub(crate) fn style(&self) -> Result<deka_native_ui::Style> {
        let mut style = if self.text.is_some() {
            deka_native_ui::Style::default()
        } else {
            // Inputs use the platform editor; their presentation container is
            // a div, while the retained store keeps the original input kind.
            deka_native_ir::element_style(if self.tag == "input" {
                "div"
            } else {
                &self.tag
            })?
        };
        deka_native_ir::apply_classes(&mut style, &self.classes)?;
        Ok(style)
    }
    pub fn into_node(self, id: String) -> Result<Node> {
        let style = self.style()?;
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
        Self::with_hosts(program, Hosts::default())
    }
    pub fn with_hosts(program: Program, hosts: Hosts) -> Result<Self> {
        let mut component = component::Component::new(program, hosts)?;
        let frame = component.render()?;
        if !frame.inputs.is_empty() {
            return Err("inputs require a platform text editor host".into());
        }
        Ok(Self {
            component,
            tree: frame.root,
        })
    }
    pub fn set_waker(&mut self, waker: &std::task::Waker) {
        self.component.set_waker(waker);
    }
    pub fn has_ready_work(&self) -> bool {
        self.component.has_ready_work()
    }
    pub fn run_turn(&mut self, cx: &mut std::task::Context<'_>, budget: usize) -> Result<bool> {
        let turn = self.component.run_turn(cx, budget)?;
        if !turn.progressed {
            return Ok(false);
        }
        let frame = self.component.render()?;
        if !frame.inputs.is_empty() {
            return Err("inputs require a platform text editor host".into());
        }
        let changed = self.tree != frame.root;
        reconcile(&mut self.tree, frame.root);
        Ok(changed)
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
// Preserve matching presentation allocations; lasting identity belongs to the
// component's Rust tree store, not to these renderer snapshots.
fn reconcile(current: &mut Node, next: Node) {
    if current.id != next.id {
        *current = next;
        return;
    }
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
struct WindowWake(deka_native_ui::Waker);
impl std::task::Wake for WindowWake {
    fn wake(self: std::sync::Arc<Self>) {
        self.0.wake();
    }
}
impl VmApp {
    pub fn set_vm_waker(&self, waker: &std::task::Waker) {
        self.session.borrow_mut().set_waker(waker);
    }
    pub fn waker(&self) -> std::task::Waker {
        self.session.borrow().component.vm_waker()
    }
    pub fn instructions(&self) -> u64 {
        self.session.borrow().instructions()
    }
    pub fn new(program: Program) -> Result<Self> {
        Self::with_hosts(program, Hosts::default())
    }
    pub fn with_hosts(program: Program, hosts: Hosts) -> Result<Self> {
        Ok(Self {
            session: RefCell::new(UiSession::with_hosts(program, hosts)?),
            error: RefCell::new(None),
        })
    }
}
impl Application for VmApp {
    fn set_waker(&mut self, waker: deka_native_ui::Waker) {
        self.set_vm_waker(&std::task::Waker::from(std::sync::Arc::new(WindowWake(
            waker,
        ))));
    }
    fn has_ready_work(&self) -> bool {
        self.error.borrow().is_none() && self.session.borrow().has_ready_work()
    }
    fn run_turn(&mut self, budget: usize) -> bool {
        if self.error.borrow().is_some() {
            return false;
        }
        let waker = self.session.borrow().component.vm_waker();
        match self
            .session
            .borrow_mut()
            .run_turn(&mut std::task::Context::from_waker(&waker), budget)
        {
            Ok(changed) => changed,
            Err(error) => {
                *self.error.borrow_mut() = Some(error);
                true
            }
        }
    }
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
