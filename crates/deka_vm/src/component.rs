//! Persistent component callbacks and dynamic DSX frames for native hosts.
use crate::{
    heap::{Dependencies, Handle, Value},
    ui::WireNode,
    *,
};
use deka_native_ui::Node;
pub(crate) mod tree;

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
struct Binding {
    value: Handle,
    dependencies: Dependencies,
    seen: bool,
}
pub struct Component {
    vm: Vm,
    instance: Handle,
    handlers: Vec<Handle>,
    evaluations: usize,
    tree: tree::Tree,
    bindings: std::collections::BTreeMap<Handle, Binding>,
    slots: Vec<Vec<usize>>,
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
            evaluations: 0,
            tree: tree::Tree::default(),
            bindings: Default::default(),
            slots: vec![],
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
            .collect::<Result<Vec<_>>>()?;
        let result = self.vm.invoke_args(method, args)?;
        self.vm.to_host(result)
    }
    pub fn event(&mut self, id: usize, args: Vec<HostValue>) -> Result<()> {
        let handler = *self.handlers.get(id).ok_or("unknown component event")?;
        let args = args
            .into_iter()
            .map(|value| self.vm.alloc_host_value(value))
            .collect::<Result<Vec<_>>>()?;
        self.vm.enqueue_event(handler, args)?;
        let waker = self.vm.waker();
        self.vm
            .run_turn(&mut std::task::Context::from_waker(&waker), 1024)?;
        Ok(())
    }
    pub fn set_waker(&mut self, waker: &std::task::Waker) {
        self.vm.set_waker(waker);
    }
    pub(crate) fn vm_waker(&self) -> std::task::Waker {
        self.vm.waker()
    }
    pub fn has_ready_work(&self) -> bool {
        self.vm.has_ready_work()
    }
    pub fn run_turn(&mut self, cx: &mut std::task::Context<'_>, budget: usize) -> Result<Turn> {
        self.vm.run_turn(cx, budget)
    }
    pub fn instructions(&self) -> u64 {
        self.vm.instructions()
    }
    pub fn evaluations(&self) -> usize {
        self.evaluations
    }
    pub fn stats(&self) -> HeapStats {
        self.vm.stats()
    }
    pub fn render(&mut self) -> Result<ComponentFrame> {
        let mut pins = vec![self.instance];
        for (getter, binding) in &mut self.bindings {
            binding.seen = false;
            pins.extend([*getter, binding.value]);
            pins.extend(binding.dependencies.roots());
        }
        self.vm.set_pins(pins);
        self.slots.clear();
        // A plain DSX root holds live binding closures. Native host integrations
        // may instead provide a record of callbacks with a view method.
        let view = if let Value::Record(fields) = self.vm.heap.get(self.instance)? {
            if fields.contains_key("tag") {
                self.instance
            } else {
                self.resolve(self.method("view")?)?
            }
        } else {
            return Err("component entry must return a view or callback record".into());
        };
        self.vm.pin(view);
        self.handlers.clear();
        let mut inputs = vec![];
        let wire = self.node(view, &mut inputs, vec![])?;
        let root = self.tree.update_slots(wire, &self.slots)?;
        self.bindings.retain(|_, binding| binding.seen);
        let mut pins = vec![self.instance, view];
        pins.extend(self.handlers.iter().copied());
        for (getter, binding) in &self.bindings {
            pins.extend([*getter, binding.value]);
            pins.extend(binding.dependencies.roots());
        }
        self.vm.set_pins(pins);
        self.vm.collect()?;
        Ok(ComponentFrame { root, inputs })
    }
    fn resolve(&mut self, value: Handle) -> Result<Handle> {
        if matches!(self.vm.heap.get(value)?, Value::Closure { .. }) {
            if let Some(binding) = self.bindings.get_mut(&value)
                && !binding.dependencies.dirty(&self.vm.heap)
            {
                binding.seen = true;
                return Ok(binding.value);
            }
            self.evaluations += 1;
            self.vm.heap.begin_reads();
            let outcome = self.vm.invoke_sync(value);
            let dependencies = self.vm.heap.end_reads();
            let result = outcome?;
            for h in dependencies.roots() {
                self.vm.pin(h);
            }
            self.bindings.insert(
                value,
                Binding {
                    value: result,
                    dependencies,
                    seen: true,
                },
            );
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
            Value::Number(n) => Ok(crate::machine::number_text(*n)),
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
    fn children(
        &mut self,
        value: Handle,
        inputs: &mut Vec<InputSpec>,
        path: Vec<usize>,
    ) -> Result<Vec<WireNode>> {
        let value = self.resolve(value)?;
        match self.vm.heap.get(value)?.clone() {
            Value::List(items) => {
                let mut children = vec![];
                for (index, item) in items.into_iter().enumerate() {
                    let mut slot = path.clone();
                    slot.push(index);
                    children.extend(self.children(item, inputs, slot)?);
                }
                Ok(children)
            }
            Value::Unit => Ok(vec![]),
            Value::Record(record) if record.enum_name.as_deref() == Some("Option")
                && record.get("name").is_some_and(|h| matches!(self.vm.heap.get(*h), Ok(Value::String(name)) if name == "None")) => Ok(vec![]),
            _ => Ok(vec![self.node(value, inputs, path)?]),
        }
    }
    fn node(
        &mut self,
        value: Handle,
        inputs: &mut Vec<InputSpec>,
        path: Vec<usize>,
    ) -> Result<WireNode> {
        let value = self.resolve(value)?;
        self.slots.push(path.clone());
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
            .map(|h| self.children(*h, inputs, path.clone()))
            .transpose()?
            .unwrap_or_default();
        Ok(WireNode {
            tag,
            classes,
            handler,
            children,
            text: None,
        })
    }
}
