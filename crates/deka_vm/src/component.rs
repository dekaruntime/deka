//! Persistent component callbacks and dynamic DSX frames for native hosts.
use crate::{
    heap::{Handle, Value},
    ui::WireNode,
    *,
};
use deka_native_ui::Node;

pub struct InputSpec {
    pub target: usize,
    pub value: String,
    pub placeholder: String,
    pub change: usize,
    pub key: usize,
}
pub struct ComponentFrame {
    pub root: Node,
    pub inputs: Vec<InputSpec>,
}
pub struct Component {
    vm: Vm,
    instance: Handle,
    handlers: Vec<Handle>,
}
impl Component {
    pub fn new(program: Program, hosts: Hosts) -> Result<Self> {
        let mut vm = Vm::new(program, hosts)?;
        let instance = vm.finish_sync()?;
        vm.pin(instance);
        if !matches!(vm.heap.get(instance)?, Value::Record(_)) {
            return Err("component entry must return a callback record".into());
        }
        Ok(Self {
            vm,
            instance,
            handlers: vec![],
        })
    }
    fn method(&self, name: &str) -> Result<Handle> {
        let Value::Record(fields) = self.vm.heap.get(self.instance)? else {
            return Err("invalid component".into());
        };
        fields
            .get(name)
            .copied()
            .ok_or_else(|| format!("missing component method {name}"))
    }
    pub fn call(&mut self, name: &str, args: Vec<HostValue>) -> Result<HostValue> {
        let method = self.method(name)?;
        let args = args
            .into_iter()
            .map(|value| self.vm.alloc_host_value(value))
            .collect();
        let result = self.vm.invoke_args(method, args)?;
        self.vm.to_host(result)
    }
    pub fn event(&mut self, id: usize, args: Vec<HostValue>) -> Result<()> {
        let handler = *self.handlers.get(id).ok_or("unknown component event")?;
        let args = args
            .into_iter()
            .map(|value| self.vm.alloc_host_value(value))
            .collect();
        self.vm.invoke_args(handler, args)?;
        Ok(())
    }
    pub fn stats(&self) -> HeapStats {
        self.vm.stats()
    }
    pub fn render(&mut self) -> Result<ComponentFrame> {
        self.vm.set_pins(vec![self.instance]);
        let view = self.vm.invoke_sync(self.method("view")?)?;
        self.vm.pin(view);
        self.handlers.clear();
        let mut inputs = vec![];
        let wire = self.node(view, &mut inputs)?;
        let root = wire.into_node("root".into())?;
        self.vm.collect()?;
        Ok(ComponentFrame { root, inputs })
    }
    fn resolve(&mut self, value: Handle) -> Result<Handle> {
        if matches!(self.vm.heap.get(value)?, Value::Closure { .. }) {
            let result = self.vm.invoke_sync(value)?;
            // Dynamic lists own event closures which must survive until the next frame.
            self.vm.pin(result);
            Ok(result)
        } else {
            Ok(value)
        }
    }
    fn text(&mut self, value: Handle) -> Result<String> {
        let value = self.resolve(value)?;
        match self.vm.heap.get(value)? {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            _ => Err("attribute must be scalar".into()),
        }
    }
    fn handler(&mut self, value: Handle) -> Result<usize> {
        if !matches!(self.vm.heap.get(value)?, Value::Closure { .. }) {
            return Err("event requires closure".into());
        }
        let id = self.handlers.len();
        self.handlers.push(value);
        Ok(id)
    }
    fn children(&mut self, value: Handle, inputs: &mut Vec<InputSpec>) -> Result<Vec<WireNode>> {
        let value = self.resolve(value)?;
        match self.vm.heap.get(value)?.clone() {
            Value::List(items) => {
                let mut children = vec![];
                for item in items {
                    children.extend(self.children(item, inputs)?);
                }
                Ok(children)
            }
            Value::Unit => Ok(vec![]),
            _ => Ok(vec![self.node(value, inputs)?]),
        }
    }
    fn node(&mut self, value: Handle, inputs: &mut Vec<InputSpec>) -> Result<WireNode> {
        let value = self.resolve(value)?;
        let Value::Record(fields) = self.vm.heap.get(value)?.clone() else {
            return Ok(WireNode {
                text: Some(self.text(value)?),
                ..Default::default()
            });
        };
        let tag = self.text(*fields.get("tag").ok_or("missing node tag")?)?;
        let classes = fields
            .get("className")
            .map(|h| self.text(*h))
            .transpose()?
            .unwrap_or_default();
        let mut handler = fields
            .get("onClick")
            .map(|h| self.handler(*h))
            .transpose()?;
        if tag == "input" {
            let target = usize::MAX - inputs.len();
            handler = Some(target);
            inputs.push(InputSpec {
                target,
                value: fields
                    .get("value")
                    .map(|h| self.text(*h))
                    .transpose()?
                    .unwrap_or_default(),
                placeholder: fields
                    .get("placeholder")
                    .map(|h| self.text(*h))
                    .transpose()?
                    .unwrap_or_default(),
                change: self.handler(*fields.get("onInput").ok_or("input requires onInput")?)?,
                key: self.handler(*fields.get("onKeyDown").ok_or("input requires onKeyDown")?)?,
            });
        }
        let children = fields
            .get("children")
            .map(|h| self.children(*h, inputs))
            .transpose()?
            .unwrap_or_default();
        Ok(WireNode {
            tag: if tag == "input" { "div".into() } else { tag },
            classes,
            handler,
            children,
            text: None,
        })
    }
}
