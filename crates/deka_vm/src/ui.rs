//! Adapter from VM-owned component instances to the existing retained Rust UI tree.
use crate::{
    heap::{Handle, Value},
    *,
};
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
struct Binding {
    path: Vec<usize>,
    closure: Handle,
}
pub struct UiSession {
    vm: Vm,
    tree: Node,
    handlers: Vec<Handle>,
    bindings: Vec<Binding>,
    evaluations: usize,
}
impl UiSession {
    pub fn new(program: Program) -> Result<Self> {
        let mut vm = Vm::new(program, Hosts::default())?;
        let root = vm.finish_sync()?;
        vm.pin(root);
        let mut handlers = vec![];
        let mut bindings = vec![];
        let wire = read_node(&mut vm, root, vec![], &mut handlers, &mut bindings)?;
        let evaluations = bindings.len();
        Ok(Self {
            vm,
            tree: wire.into_node("root".into())?,
            handlers,
            bindings,
            evaluations,
        })
    }
    pub fn tree(&self) -> &Node {
        &self.tree
    }
    pub fn stats(&self) -> HeapStats {
        self.vm.stats()
    }
    pub fn instructions(&self) -> u64 {
        self.vm.instructions()
    }
    pub fn evaluations(&self) -> usize {
        self.evaluations
    }
    pub fn click(&mut self, handler: usize) -> Result<()> {
        let closure = *self.handlers.get(handler).ok_or("unknown UI handler")?;
        self.vm.invoke_sync(closure)?;
        // This first VM adapter reevaluates binding closures after events. It does not
        // rerun the component or recreate nodes. Dependency indexing is future work.
        for binding in &self.bindings {
            let value = self.vm.invoke_sync(binding.closure)?;
            let text = text_value(&self.vm, value)?;
            let mut node = &mut self.tree;
            for index in &binding.path {
                node = &mut node.children[*index];
            }
            if node.text.as_ref() != Some(&text) {
                node.text = Some(text);
            }
            self.evaluations += 1;
        }
        self.vm.collect()
    }
}
fn text_value(vm: &Vm, h: Handle) -> Result<String> {
    match vm.heap.get(h)? {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err("UI text bindings must produce strings, numbers or bools".into()),
    }
}
fn read_node(
    vm: &mut Vm,
    h: Handle,
    path: Vec<usize>,
    handlers: &mut Vec<Handle>,
    bindings: &mut Vec<Binding>,
) -> Result<WireNode> {
    match vm.heap.get(h)?.clone() {
        Value::String(s) => Ok(WireNode {
            text: Some(s),
            ..Default::default()
        }),
        Value::Closure { .. } => {
            let result = vm.invoke_sync(h)?;
            let text = text_value(vm, result)?;
            bindings.push(Binding { path, closure: h });
            Ok(WireNode {
                text: Some(text),
                ..Default::default()
            })
        }
        Value::Record(fields) => {
            let get_string = |name: &str| -> Result<String> {
                fields
                    .get(name)
                    .map(|h| text_value(vm, *h))
                    .unwrap_or(Ok(String::new()))
            };
            let tag = get_string("tag")?;
            let classes = get_string("className")?;
            let handler = if let Some(h) = fields.get("onClick") {
                if !matches!(vm.heap.get(*h)?, Value::Closure { .. }) {
                    return Err("onClick requires a VM closure".into());
                }
                let index = handlers.len();
                handlers.push(*h);
                Some(index)
            } else {
                None
            };
            let children = fields
                .get("children")
                .ok_or("UI node has no children list")?;
            let Value::List(children) = vm.heap.get(*children)?.clone() else {
                return Err("UI children must be a list".into());
            };
            let mut nodes = vec![];
            for (i, child) in children.into_iter().enumerate() {
                let mut path = path.clone();
                path.push(i);
                nodes.push(read_node(vm, child, path, handlers, bindings)?);
            }
            Ok(WireNode {
                tag,
                classes,
                text: None,
                handler,
                children: nodes,
            })
        }
        _ => Err("component did not return a UI node".into()),
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
