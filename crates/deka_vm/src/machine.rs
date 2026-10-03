use crate::{
    heap::{Handle, Heap, Outcome, Value},
    stack::Stack,
    *,
};
use std::{
    collections::BTreeMap,
    rc::Rc,
    task::{Context, Poll},
};
struct Frame {
    function: usize,
    ip: usize,
    locals: Vec<Handle>,
    stack: Stack,
    slot_children: Option<Handle>,
    /// Set on a component-prop getter frame read as a call's callee
    /// (`props.onSelect()`): on return, the getter's result is called with
    /// these arguments instead of being handed to the caller.
    then_call: Option<Vec<Handle>>,
    handlers: Vec<Handler>,
}
struct Handler {
    target: usize,
    stack_depth: usize,
}
enum Work {
    Code(Vec<Frame>),
    Host {
        future: HostFuture,
        result: HostType,
        result_channel: bool,
    },
}
struct Task {
    promise: Handle,
    work: Work,
}
enum Step {
    Continue,
    Blocked,
    Complete(Outcome),
}
/// What a call setup produced: a frame to enter or a spawned task's promise.
enum Invocation {
    Frame(Frame),
    Task(Handle),
}
pub struct Vm {
    program: Rc<Program>,
    hosts: Hosts,
    pub(crate) heap: Heap,
    pins: Vec<Handle>,
    tasks: BTreeMap<u64, Task>,
    next_task: u64,
    root: Option<Handle>,
    instructions: u64,
    instruction_limit: u64,
    descriptors: BTreeMap<TypeDescriptor, Handle>,
}
impl Vm {
    pub fn new(program: Program, hosts: Hosts) -> Result<Self> {
        program.validate()?;
        if program.functions[0].parameters != 0 || program.functions[0].captures != 0 {
            return Err("entry function cannot take arguments/captures".into());
        }
        let mut vm = Self {
            program: Rc::new(program),
            hosts,
            heap: Heap::default(),
            pins: vec![],
            tasks: BTreeMap::new(),
            next_task: 0,
            root: None,
            instructions: 0,
            instruction_limit: 10_000_000,
            descriptors: BTreeMap::new(),
        };
        let frame = vm.frame(0, vec![], vec![], None)?;
        vm.root = Some(vm.spawn(Work::Code(vec![frame])));
        Ok(vm)
    }
    pub fn stats(&self) -> HeapStats {
        self.heap.stats()
    }
    pub fn instructions(&self) -> u64 {
        self.instructions
    }
    pub fn set_instruction_limit(&mut self, limit: u64) {
        self.instruction_limit = limit;
    }
    pub fn pending_tasks(&self) -> usize {
        self.tasks.len()
    }
    pub fn cancel(&mut self) -> Result<()> {
        self.tasks.clear();
        self.pins.clear();
        self.descriptors.clear();
        self.root = None;
        self.heap.collect([])
    }
    pub fn collect(&mut self) -> Result<()> {
        let mut roots: Vec<_> = self
            .root
            .into_iter()
            .chain(self.pins.iter().copied())
            .collect();
        roots.extend(self.descriptors.values().copied());
        for task in self.tasks.values() {
            roots.push(task.promise);
            if let Work::Code(frames) = &task.work {
                for f in frames {
                    roots.extend(&f.locals);
                    roots.extend(f.stack.roots());
                    roots.extend(f.slot_children);
                    roots.extend(f.then_call.iter().flatten());
                }
            }
        }
        self.heap.collect(roots)
    }
    fn spawn(&mut self, work: Work) -> Handle {
        let promise = self.heap.alloc(Value::Promise(None));
        let id = self.next_task;
        self.next_task += 1;
        self.tasks.insert(id, Task { promise, work });
        promise
    }
    fn frame(
        &mut self,
        function: usize,
        captures: Vec<Handle>,
        args: Vec<Handle>,
        slot_children: Option<Handle>,
    ) -> Result<Frame> {
        let f = self
            .program
            .functions
            .get(function)
            .ok_or("unknown function")?;
        // Fewer arguments than parameters is not an error: the missing cells
        // are unit-filled below, and a default parameter's prologue fills
        // them (JumpIfUnit).
        if args.len() > f.parameters || captures.len() != f.captures {
            return Err("function arity mismatch".into());
        }
        let mut locals = captures;
        for arg in args {
            locals.push(self.heap.alloc(Value::Cell(arg)));
        }
        while locals.len() < f.locals {
            let unit = self.heap.alloc(Value::Unit);
            locals.push(self.heap.alloc(Value::Cell(unit)));
        }
        Ok(Frame {
            function,
            ip: 0,
            locals,
            stack: Stack::new(),
            slot_children,
            then_call: None,
            handlers: vec![],
        })
    }
    pub(crate) fn to_host(&self, h: Handle) -> Result<HostValue> {
        Ok(match self.heap.get(h)? {
            Value::Unit => HostValue::Unit,
            Value::Number(n) => HostValue::Number(*n),
            Value::Bool(b) => HostValue::Bool(*b),
            Value::String(s) => HostValue::String(s.clone()),
            Value::List(items) => HostValue::Strings(
                items
                    .iter()
                    .map(|h| match self.heap.get(*h)? {
                        Value::String(s) => Ok(s.clone()),
                        _ => Err("host list values must contain strings".into()),
                    })
                    .collect::<Result<_>>()?,
            ),
            _ => return Err("unsupported host wire value".into()),
        })
    }
    fn host_result(
        &mut self,
        value: Result<HostValue>,
        expected: HostType,
        result_channel: bool,
    ) -> Result<Handle> {
        let value = match value {
            Ok(value) => value,
            Err(error) if result_channel => {
                let payload = self.heap.alloc(Value::String(error));
                return Ok(self.enum_value("Result".into(), "Err".into(), 1, Some(payload)));
            }
            Err(error) => return Err(error),
        };
        if !expected.accepts(&value) {
            return Err("host returned the wrong result type".into());
        }
        let payload = self.alloc_host_value(value);
        Ok(if result_channel {
            self.enum_value("Result".into(), "Ok".into(), 0, Some(payload))
        } else {
            payload
        })
    }
    pub(crate) fn alloc_host_value(&mut self, value: HostValue) -> Handle {
        let value = match value {
            HostValue::Unit => Value::Unit,
            HostValue::Number(n) => Value::Number(n),
            HostValue::Bool(b) => Value::Bool(b),
            HostValue::String(s) => Value::String(s),
            HostValue::Strings(items) => Value::List(
                items
                    .into_iter()
                    .map(|s| self.heap.alloc(Value::String(s)))
                    .collect(),
            ),
        };
        self.heap.alloc(value)
    }
    pub async fn run(&mut self) -> Result<HostValue> {
        std::future::poll_fn(|cx| self.poll(cx)).await
    }
    /// Poll runnable tasks in bounded slices, draining them after main returns.
    /// Host futures use the caller's waker; an idle poll does not mean completion.
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<HostValue>> {
        match self.poll_inner(cx, true) {
            Poll::Ready(result) => {
                let result = result.and_then(|h| self.to_host(h));
                let _ = self.cancel();
                Poll::Ready(result)
            }
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_inner(&mut self, cx: &mut Context<'_>, drain_tasks: bool) -> Poll<Result<Handle>> {
        let Some(root) = self.root else {
            return Poll::Ready(Err("VM cancelled".into()));
        };
        let mut progressed = false;
        let ids: Vec<_> = self.tasks.keys().copied().collect();
        for id in ids {
            let mut task = self.tasks.remove(&id).unwrap();
            let result = match &mut task.work {
                Work::Host {
                    future,
                    result,
                    result_channel,
                } => match future.as_mut().poll(cx) {
                    Poll::Ready(v) => Some(
                        self.host_result(v, *result, *result_channel)
                            .map(Outcome::Value),
                    ),
                    Poll::Pending => None,
                },
                Work::Code(frames) => {
                    let mut completion = None;
                    for _ in 0..256 {
                        self.instructions += 1;
                        if self.instructions > self.instruction_limit {
                            return Poll::Ready(Err("instruction limit exceeded".into()));
                        }
                        match self.step(frames) {
                            Ok(Step::Continue) => progressed = true,
                            Ok(Step::Blocked) => break,
                            Ok(Step::Complete(value)) => {
                                completion = Some(Ok(value));
                                break;
                            }
                            Err(e) => {
                                completion = Some(Err(e));
                                break;
                            }
                        }
                    }
                    completion
                }
            };
            if let Some(result) = result {
                // VM faults are fatal even when their task was not awaited.
                // Language Throw is an Outcome, so it still travels through await.
                let result = match result {
                    Ok(value) => Ok(value),
                    Err(error) => return Poll::Ready(Err(error)),
                };
                progressed = true;
                if let Err(e) = self
                    .heap
                    .replace(task.promise, Value::Promise(Some(result)))
                {
                    return Poll::Ready(Err(e));
                }
            } else {
                self.tasks.insert(id, task);
            }
        }
        // Collection only occurs at a safepoint, with every suspended frame registered.
        if let Err(e) = self.collect() {
            return Poll::Ready(Err(e));
        }
        match self.heap.get(root) {
            Ok(Value::Promise(Some(Ok(Outcome::Value(_)))))
                if drain_tasks && !self.tasks.is_empty() =>
            {
                if progressed {
                    cx.waker().wake_by_ref();
                }
                Poll::Pending
            }
            Ok(Value::Promise(Some(Ok(Outcome::Value(h))))) => Poll::Ready(Ok(*h)),
            Ok(Value::Promise(Some(Ok(Outcome::Thrown(h))))) => {
                Poll::Ready(Err(format!("uncaught Throw: {}", self.value_text(*h)?)))
            }
            Ok(Value::Promise(Some(Err(e)))) => Poll::Ready(Err(e.clone())),
            Ok(Value::Promise(None)) => {
                if progressed {
                    cx.waker().wake_by_ref();
                }
                Poll::Pending
            }
            _ => Poll::Ready(Err("invalid entry promise".into())),
        }
    }
    #[cfg(feature = "ui")]
    pub(crate) fn finish_sync(&mut self) -> Result<Handle> {
        let mut cx = Context::from_waker(std::task::Waker::noop());
        loop {
            // A synchronous UI invocation reads its own result. Desktop task
            // lifetime is managed separately from CLI program draining.
            match self.poll_inner(&mut cx, false) {
                Poll::Ready(result) => {
                    self.tasks.clear();
                    // Leave the result promise rooted until the caller pins or reads it.
                    return result;
                }
                Poll::Pending => {
                    if self
                        .tasks
                        .values()
                        .any(|t| matches!(t.work, Work::Host { .. }))
                    {
                        self.tasks.clear();
                        self.root = None;
                        self.collect()?;
                        return Err("this UI adapter currently supports synchronous handlers and bindings only".into());
                    }
                }
            }
        }
    }
    #[cfg(feature = "ui")]
    pub(crate) fn pin(&mut self, handle: Handle) {
        self.pins.push(handle);
    }
    #[cfg(feature = "ui")]
    pub(crate) fn set_pins(&mut self, pins: Vec<Handle>) {
        self.pins = pins;
    }
    #[cfg(feature = "ui")]
    pub(crate) fn invoke_sync(&mut self, closure: Handle) -> Result<Handle> {
        self.invoke_args(closure, vec![])
    }
    #[cfg(feature = "ui")]
    pub(crate) fn invoke_args(&mut self, closure: Handle, args: Vec<Handle>) -> Result<Handle> {
        // Each GUI event has its own bounded budget; cumulative instructions remain measurable.
        self.instruction_limit = self.instructions.saturating_add(10_000_000);
        let Value::Closure {
            function,
            captures,
            slot_children,
        } = self.heap.get(closure)?.clone()
        else {
            return Err("UI handler is not a closure".into());
        };
        if self.program.functions[function].asynchronous {
            return Err("async UI callbacks are not supported yet".into());
        }
        let frame = self.frame(function, captures, args, slot_children)?;
        self.root = Some(self.spawn(Work::Code(vec![frame])));
        self.finish_sync()
    }
    fn raise(&mut self, frames: &mut Vec<Frame>, value: Handle) -> Result<Step> {
        while let Some(frame) = frames.last_mut() {
            if let Some(handler) = frame.handlers.pop() {
                frame.stack.truncate(handler.stack_depth);
                frame.stack.push(value);
                frame.ip = handler.target;
                return Ok(Step::Continue);
            }
            frames.pop();
        }
        Ok(Step::Complete(Outcome::Thrown(value)))
    }
    fn step(&mut self, frames: &mut Vec<Frame>) -> Result<Step> {
        let frame = frames.last_mut().ok_or("empty call stack")?;
        let op = self.program.functions[frame.function]
            .code
            .get(frame.ip)
            .ok_or("instruction pointer outside function")?
            .clone();
        frame.ip += 1;
        match op {
            Op::Const(v) => {
                frame.stack.push(self.heap.alloc(v.into()));
            }
            Op::Pop => {
                pop(frame)?;
            }
            Op::Dup => {
                let h = pop(frame)?;
                frame.stack.push(h);
                frame.stack.push(h);
            }
            Op::Load(i) => {
                let Value::Cell(h) = self.heap.get(frame.locals[i])? else {
                    return Err("invalid local cell".into());
                };
                frame.stack.push(*h);
            }
            Op::LoadChecked { slot, message } => {
                let Value::Cell(h) = self.heap.get(frame.locals[slot])? else {
                    return Err("invalid local cell".into());
                };
                if matches!(self.heap.get(*h)?, Value::Uninitialized) {
                    return Err(message.clone());
                }
                frame.stack.push(*h);
            }
            Op::Store(i) => {
                let h = pop(frame)?;
                self.heap.replace(frame.locals[i], Value::Cell(h))?;
            }
            Op::Rebind(i) => {
                let unit = self.heap.alloc(Value::Unit);
                let cell = self.heap.alloc(Value::Cell(unit));
                frame.locals[i] = cell;
            }
            Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Mod
            | Op::Less
            | Op::LessEq
            | Op::Greater
            | Op::GreaterEq
            | Op::Equal
            | Op::NotEqual
            | Op::BitAnd
            | Op::BitOr
            | Op::BitXor
            | Op::Shl
            | Op::Shr => {
                let b = self.heap.get(pop(frame)?)?.clone();
                let a = self.heap.get(pop(frame)?)?.clone();
                let value = match (a, b) {
                    (Value::Number(a), Value::Number(b)) => match op {
                        Op::Add => Value::Number(a + b),
                        Op::Sub => Value::Number(a - b),
                        Op::Mul => Value::Number(a * b),
                        Op::Div => Value::Number(a / b),
                        Op::Mod => Value::Number(a % b),
                        Op::Less => Value::Bool(a < b),
                        Op::LessEq => Value::Bool(a <= b),
                        Op::Greater => Value::Bool(a > b),
                        Op::GreaterEq => Value::Bool(a >= b),
                        Op::Equal => Value::Bool(a == b),
                        Op::NotEqual => Value::Bool(a != b),
                        Op::BitAnd => Value::Number((int32(a) & int32(b)) as f64),
                        Op::BitOr => Value::Number((int32(a) | int32(b)) as f64),
                        Op::BitXor => Value::Number((int32(a) ^ int32(b)) as f64),
                        Op::Shl => Value::Number((int32(a) << (int32(b) & 31)) as f64),
                        Op::Shr => Value::Number((int32(a) >> (int32(b) & 31)) as f64),
                        _ => unreachable!(),
                    },
                    (Value::String(a), Value::String(b)) => match op {
                        Op::Add => Value::String(a + &b),
                        Op::Equal => Value::Bool(a == b),
                        Op::NotEqual => Value::Bool(a != b),
                        Op::Less => Value::Bool(a < b),
                        Op::LessEq => Value::Bool(a <= b),
                        Op::Greater => Value::Bool(a > b),
                        Op::GreaterEq => Value::Bool(a >= b),
                        _ => return Err("unsupported string operation".into()),
                    },
                    (Value::String(a), Value::Number(b)) if matches!(op, Op::Add) => {
                        Value::String(a + &number_text(b))
                    }
                    (Value::Number(a), Value::String(b)) if matches!(op, Op::Add) => {
                        Value::String(number_text(a) + &b)
                    }
                    (Value::String(a), Value::Bool(b)) if matches!(op, Op::Add) => {
                        Value::String(a + &b.to_string())
                    }
                    (Value::Bool(a), Value::String(b)) if matches!(op, Op::Add) => {
                        Value::String(a.to_string() + &b)
                    }
                    (Value::Descriptor(a), Value::Descriptor(b))
                        if matches!(op, Op::Equal | Op::NotEqual) =>
                    {
                        Value::Bool(if matches!(op, Op::Equal) {
                            a == b
                        } else {
                            a != b
                        })
                    }
                    (Value::Bool(a), Value::Bool(b)) if matches!(op, Op::Equal | Op::NotEqual) => {
                        Value::Bool(if matches!(op, Op::Equal) {
                            a == b
                        } else {
                            a != b
                        })
                    }
                    _ => return Err("invalid arithmetic operands".into()),
                };
                frame.stack.push(self.heap.alloc(value));
            }
            Op::Neg => {
                let h = pop(frame)?;
                let Value::Number(n) = self.heap.get(h)? else {
                    return Err("unary - requires a number".into());
                };
                let n = *n;
                frame.stack.push(self.heap.alloc(Value::Number(-n)));
            }
            Op::Not => {
                let h = pop(frame)?;
                let Value::Bool(b) = self.heap.get(h)? else {
                    return Err("! requires a bool".into());
                };
                let b = *b;
                frame.stack.push(self.heap.alloc(Value::Bool(!b)));
            }
            Op::Jump(ip) => frame.ip = ip,
            Op::Newtype(name) => {
                let value = self.heap.get(pop(frame)?)?.clone();
                frame.stack.push(self.heap.alloc_newtype(value, name));
            }
            Op::GetType => {
                let value = pop(frame)?;
                let descriptor = self.type_of(value)?;
                let h = self.intern_descriptor(descriptor);
                frame.stack.push(h);
            }
            Op::Descriptor(descriptor) => {
                let h = self.intern_descriptor(descriptor);
                frame.stack.push(h);
            }
            Op::Enum {
                name,
                case,
                index,
                payload,
            } => {
                let value = if payload { Some(pop(frame)?) } else { None };
                let h = self.enum_value(name, case, index, value);
                frame.stack.push(h);
            }
            Op::MatchEnum { name, case } => {
                let h = pop(frame)?;
                let matched = match self.heap.get(h)? {
                    Value::Record(record) if record.enum_name.is_some()
                        && name.as_ref().is_none_or(|n| record.enum_name.as_ref() == Some(n)) => {
                        record.get("name").is_some_and(|label| matches!(self.heap.get(*label), Ok(Value::String(label)) if label == &case))
                    }
                    _ => false,
                };
                frame.stack.push(self.heap.alloc(Value::Bool(matched)));
            }
            Op::MatchStruct(identity) => {
                let h = pop(frame)?;
                let matched = matches!(self.heap.get(h)?, Value::Record(record) if record.struct_identity.as_ref() == Some(&identity));
                frame.stack.push(self.heap.alloc(Value::Bool(matched)));
            }
            Op::MatchType(expected) => {
                let h = pop(frame)?;
                let matched = self.type_of(h)? == expected;
                frame.stack.push(self.heap.alloc(Value::Bool(matched)));
            }
            Op::MatchEqual => {
                let b = pop(frame)?;
                let a = pop(frame)?;
                let matched = match (self.heap.get(a)?, self.heap.get(b)?) {
                    (Value::Number(a), Value::Number(b)) => a == b,
                    (Value::String(a), Value::String(b)) => a == b,
                    (Value::Bool(a), Value::Bool(b)) => a == b,
                    (Value::Unit, Value::Unit) => true,
                    _ => false,
                };
                frame.stack.push(self.heap.alloc(Value::Bool(matched)));
            }
            Op::MatchTuple(length) => {
                let matched = matches!(self.heap.get(pop(frame)?)?, Value::List(items) if items.len() == length);
                frame.stack.push(self.heap.alloc(Value::Bool(matched)));
            }
            Op::ToString => {
                let h = pop(frame)?;
                let text = self.value_text(h)?;
                let value = self.heap.alloc(Value::String(text));
                frame.stack.push(value);
            }
            Op::ToNumber => {
                let h = pop(frame)?;
                let value = match self.heap.get(h)? {
                    Value::Number(n) => self.heap.alloc(Value::Number(*n)),
                    Value::Bool(b) => self.heap.alloc(Value::Number(if *b { 1. } else { 0. })),
                    _ => return Err("toNumber() only accepts number or bool".into()),
                };
                frame.stack.push(value);
            }
            Op::Panic => {
                let h = pop(frame)?;
                let Value::String(message) = self.heap.get(h)? else {
                    return Err("panic() requires a string message".into());
                };
                return Err(message.clone());
            }
            Op::JumpIfFalse(ip) => {
                let Value::Bool(b) = self.heap.get(pop(frame)?)? else {
                    return Err("condition must be bool".into());
                };
                if !b {
                    frame.ip = ip;
                }
            }
            Op::JumpIfUnit(ip) => {
                if matches!(self.heap.get(pop(frame)?)?, Value::Unit) {
                    frame.ip = ip;
                }
            }
            Op::Closure { function, captures } => {
                let captures = captures.into_iter().map(|i| frame.locals[i]).collect();
                let slot_children = frame.slot_children;
                frame.stack.push(self.heap.alloc(Value::Closure {
                    function,
                    captures,
                    slot_children,
                }));
            }
            Op::Slot => {
                let children = frame
                    .slot_children
                    .unwrap_or_else(|| self.heap.alloc(Value::Unit));
                frame.stack.push(children);
            }
            Op::ComponentCall => {
                let props = pop(frame)?;
                let Value::Closure {
                    function, captures, ..
                } = self.heap.get(pop(frame)?)?.clone()
                else {
                    return Err("component is not callable".into());
                };
                let slot_children = match self.heap.get(props)? {
                    Value::Props(fields) => fields.get("children").copied(),
                    _ => return Err("component call requires a props record".into()),
                };
                let target = &self.program.functions[function];
                if target.asynchronous || target.parameters > 1 {
                    return Err(
                        "component must be synchronous with zero or one props parameter".into(),
                    );
                }
                let args = if target.parameters == 0 {
                    vec![]
                } else {
                    vec![props]
                };
                let next = self.frame(function, captures, args, slot_children)?;
                if frames.len() >= 1024 {
                    return Err("call stack limit exceeded".into());
                }
                frames.push(next);
            }
            Op::Call(argc) => {
                let args = arguments(frame, argc)?;
                let Value::Closure {
                    function,
                    captures,
                    slot_children,
                } = self.heap.get(pop(frame)?)?.clone()
                else {
                    return Err("value is not callable".into());
                };
                match self.invoke(function, captures, args, slot_children)? {
                    Invocation::Task(h) => frame.stack.push(h),
                    Invocation::Frame(next) => {
                        if frames.len() >= 1024 {
                            return Err("call stack limit exceeded".into());
                        }
                        frames.push(next);
                    }
                }
            }
            Op::MethodCall { name, argc } => {
                let args = arguments(frame, argc)?;
                let receiver = pop(frame)?;
                // Component props carry no attached methods: the member is a
                // prop getter whose current value is the function to call.
                if let Value::Props(fields) = self.heap.get(receiver)? {
                    let value = *fields.get(name.as_str()).ok_or("missing field")?;
                    self.read_prop(frames, Some(value), Some(args))?;
                    return Ok(Step::Continue);
                }
                if let Value::Descriptor(descriptor) = self.heap.get(receiver)?
                    && name == "toString"
                    && args.is_empty()
                {
                    let text = descriptor.name.clone();
                    frame.stack.push(self.heap.alloc(Value::String(text)));
                    return Ok(Step::Continue);
                }
                let Value::Record(fields) = self.heap.get(receiver)? else {
                    return Err("method call requires a record".into());
                };
                let hidden = format!("${name}");
                let (callee, with_receiver) = if let Some(f) = fields.get(&hidden) {
                    (*f, true)
                } else if let Some(f) = fields.get(name.as_str()) {
                    (*f, false)
                } else {
                    return Err("missing field".into());
                };
                let Value::Closure {
                    function,
                    captures,
                    slot_children,
                } = self.heap.get(callee)?.clone()
                else {
                    return Err("value is not callable".into());
                };
                let mut all = Vec::with_capacity(args.len() + 1);
                if with_receiver {
                    all.push(receiver);
                }
                all.extend(args);
                match self.invoke(function, captures, all, slot_children)? {
                    Invocation::Task(h) => frame.stack.push(h),
                    Invocation::Frame(next) => {
                        if frames.len() >= 1024 {
                            return Err("call stack limit exceeded".into());
                        }
                        frames.push(next);
                    }
                }
            }
            Op::Host {
                operation,
                arguments: argc,
            } => {
                let args = arguments(frame, argc)?
                    .into_iter()
                    .map(|h| self.to_host(h))
                    .collect::<Result<Vec<_>>>()?;
                let (reply, expected, asynchronous, result_channel) =
                    self.hosts.call(&operation, args)?;
                let h = match reply {
                    HostReply::Pending(future) => self.spawn(Work::Host {
                        future,
                        result: expected,
                        result_channel,
                    }),
                    HostReply::Ready(value) => {
                        let result = self.host_result(value, expected, result_channel);
                        if asynchronous {
                            self.heap
                                .alloc(Value::Promise(Some(result.map(Outcome::Value))))
                        } else {
                            result?
                        }
                    }
                };
                frame.stack.push(h);
            }
            Op::Await => {
                let promise = pop(frame)?;
                match self.heap.get(promise)? {
                    Value::Promise(Some(result)) => match result.clone()? {
                        Outcome::Value(h) => frame.stack.push(h),
                        Outcome::Thrown(h) => return self.raise(frames, h),
                    },
                    Value::Promise(None) => {
                        frame.stack.push(promise);
                        frame.ip -= 1;
                        return Ok(Step::Blocked);
                    }
                    _ => return Err("await requires a promise".into()),
                }
            }
            Op::Handler(target) => frame.handlers.push(Handler {
                target,
                stack_depth: frame.stack.len(),
            }),
            Op::EndHandler => {
                frame.handlers.pop().ok_or("missing exception handler")?;
            }
            Op::Throw => {
                let value = pop(frame)?;
                return self.raise(frames, value);
            }
            Op::Return => {
                let result = pop(frame)?;
                let finished = frames.pop().ok_or("empty call stack")?;
                if let Some(args) = finished.then_call {
                    self.call_value(frames, result, args)?;
                    return Ok(Step::Continue);
                }
                if let Some(parent) = frames.last_mut() {
                    parent.stack.push(result);
                } else {
                    return Ok(Step::Complete(Outcome::Value(result)));
                }
            }
            Op::List(count) => {
                let items = arguments(frame, count)?;
                frame.stack.push(self.heap.alloc(Value::List(items)));
            }
            Op::ListHas => {
                let index = pop(frame)?;
                let list = pop(frame)?;
                let Value::Number(index) = self.heap.get(index)? else {
                    return Err("index must be number".into());
                };
                let Value::List(items) = self.heap.get(list)? else {
                    return Err("has requires list".into());
                };
                let valid = index.is_finite()
                    && *index >= 0.
                    && index.fract() == 0.
                    && *index < items.len() as f64;
                frame.stack.push(self.heap.alloc(Value::Bool(valid)));
            }
            Op::ListAppend => {
                let item = pop(frame)?;
                let list = pop(frame)?;
                let Value::List(mut items) = self.heap.get(list)?.clone() else {
                    return Err("append requires list".into());
                };
                items.push(item);
                frame.stack.push(self.heap.alloc(Value::List(items)));
            }
            Op::Props(names) => {
                let items = arguments(frame, names.len())?;
                frame.stack.push(
                    self.heap
                        .alloc(Value::Props(names.into_iter().zip(items).collect())),
                );
            }
            Op::ListExtend => {
                let source = pop(frame)?;
                let target = pop(frame)?;
                let Value::List(extra) = self.heap.get(source)?.clone() else {
                    return Err("spread requires a list".into());
                };
                let Value::List(mut items) = self.heap.get(target)?.clone() else {
                    return Err("extend requires list".into());
                };
                items.extend(extra);
                frame.stack.push(self.heap.alloc(Value::List(items)));
            }
            Op::Record(names) => {
                let items = arguments(frame, names.len())?;
                frame.stack.push(
                    self.heap
                        .alloc(Value::Record(names.into_iter().zip(items).collect())),
                );
            }
            Op::Struct {
                name,
                identity,
                fields,
                embeds,
            } => {
                let items = arguments(frame, fields.len())?;
                frame
                    .stack
                    .push(self.heap.alloc(Value::Record(crate::heap::Record {
                        order: fields.clone(),
                        fields: fields.into_iter().zip(items).collect(),
                        struct_name: Some(name),
                        struct_identity: identity,
                        enum_name: None,
                        embeds,
                    })));
            }
            Op::RecordExtend => {
                let source = pop(frame)?;
                let target = pop(frame)?;
                let Value::Record(extra) = self.heap.get(source)?.clone() else {
                    return Err("object spread requires a record".into());
                };
                let Value::Record(mut fields) = self.heap.get(target)?.clone() else {
                    return Err("extend requires record".into());
                };
                for name in extra.order {
                    if !fields.contains_key(&name) {
                        fields.order.push(name.clone());
                    }
                    fields.insert(name.clone(), extra.fields[&name]);
                }
                frame.stack.push(self.heap.alloc(Value::Record(fields)));
            }
            Op::Field(name) => {
                let h = pop(frame)?;
                if let Value::Props(fields) = self.heap.get(h)? {
                    let value = fields.get(&name).copied();
                    self.read_prop(frames, value, None)?;
                    return Ok(Step::Continue);
                }
                let value = match self.heap.get(h)? {
                    Value::List(items) if name == "length" => {
                        self.heap.alloc(Value::Number(items.len() as f64))
                    }
                    Value::String(text) if name == "length" => {
                        self.heap.alloc(Value::Number(text.chars().count() as f64))
                    }
                    Value::Record(_) => {
                        let owner = self.field_owner(h, &name)?.ok_or("missing field")?;
                        let Value::Record(fields) = self.heap.get(owner)? else {
                            unreachable!()
                        };
                        *fields.get(&name).ok_or("missing field")?
                    }
                    _ => return Err("unsupported field access".into()),
                };
                frame.stack.push(value);
            }
            Op::FieldOrSelf(name) => {
                let h = pop(frame)?;
                if let Value::Props(fields) = self.heap.get(h)?
                    && let Some(value) = fields.get(&name).copied()
                {
                    self.read_prop(frames, Some(value), None)?;
                    return Ok(Step::Continue);
                }
                let value = match self.heap.get(h)? {
                    Value::Props(_) => h,
                    Value::Record(fields) => fields.get(&name).copied().unwrap_or(h),
                    _ => return Err("unsupported field access".into()),
                };
                frame.stack.push(value);
            }
            Op::Index => {
                let index = pop(frame)?;
                let object = pop(frame)?;
                let Value::Number(index) = self.heap.get(index)? else {
                    return Err("index must be number".into());
                };
                if !index.is_finite() || *index < 0. || index.fract() != 0. {
                    return Err("invalid index".into());
                }
                let index = *index as usize;
                match self.heap.get(object)? {
                    Value::List(items) => {
                        frame
                            .stack
                            .push(*items.get(index).ok_or("index out of bounds")?);
                    }
                    Value::String(text) => {
                        let ch = text
                            .chars()
                            .nth(index)
                            .ok_or("index out of bounds")?
                            .to_string();
                        let h = self.heap.alloc(Value::String(ch));
                        frame.stack.push(h);
                    }
                    _ => return Err("index requires a list or string".into()),
                }
            }
            Op::FieldSet(name) => {
                let value = pop(frame)?;
                let object = pop(frame)?;
                let object = self.field_owner(object, &name)?.unwrap_or(object);
                match self.heap.get_mut(object)? {
                    Value::Record(fields) => {
                        if !fields.contains_key(&name) {
                            fields.order.push(name.clone());
                        }
                        fields.insert(name.clone(), value);
                    }
                    _ => return Err("Cannot assign to read only property".into()),
                }
                frame.stack.push(value);
            }
            Op::IndexSet => {
                let value = pop(frame)?;
                let index = pop(frame)?;
                let object = pop(frame)?;
                let Value::Number(index) = self.heap.get(index)? else {
                    return Err("index must be number".into());
                };
                if !index.is_finite() || *index < 0. || index.fract() != 0. {
                    return Err("invalid index".into());
                }
                let index = *index as usize;
                match self.heap.get_mut(object)? {
                    Value::List(items) => {
                        *items.get_mut(index).ok_or("index out of bounds")? = value;
                    }
                    Value::String(_) => {
                        return Err("Cannot assign to read only property".into());
                    }
                    _ => return Err("index assignment requires a list".into()),
                }
                frame.stack.push(value);
            }
            Op::ListMut(kind) => {
                self.list_mut(frame, &kind)?;
            }
        }
        Ok(Step::Continue)
    }
    fn intern_descriptor(&mut self, descriptor: TypeDescriptor) -> Handle {
        if let Some(h) = self.descriptors.get(&descriptor) {
            return *h;
        }
        let h = self.heap.alloc(Value::Descriptor(descriptor.clone()));
        self.descriptors.insert(descriptor, h);
        h
    }
    fn type_of(&self, value: Handle) -> Result<TypeDescriptor> {
        if let Some(name) = self.heap.newtype_name(value)? {
            return Ok(TypeDescriptor::new("newtype", name));
        }
        Ok(match self.heap.get(value)? {
            Value::Unit => TypeDescriptor::new("none", "None"),
            Value::Number(_) => TypeDescriptor::new("number", "number"),
            Value::Bool(_) => TypeDescriptor::new("boolean", "boolean"),
            Value::String(_) => TypeDescriptor::new("string", "string"),
            Value::List(_) => TypeDescriptor::new("array", "Array"),
            Value::Record(record) => {
                if let Some(name) = &record.struct_name {
                    TypeDescriptor::new("struct", name)
                } else if let Some(name) = &record.enum_name {
                    TypeDescriptor::new("enum", name)
                } else {
                    TypeDescriptor::new("object", "Object")
                }
            }
            Value::Closure { .. } => TypeDescriptor::new("function", "Function"),
            Value::Descriptor(_) => TypeDescriptor::new("object", "Type"),
            Value::Promise(_) => TypeDescriptor::new("object", "Promise"),
            Value::Props(_) => TypeDescriptor::new("object", "Object"),
            _ => return Err("uninitialized value has no runtime type".into()),
        })
    }
    /// Declared and prelude enums use this one nominal constructor. Field
    /// storage remains compatible with existing case/payload access.
    fn enum_value(
        &mut self,
        name: String,
        case: String,
        index: usize,
        value: Option<Handle>,
    ) -> Handle {
        let label = self.heap.alloc(Value::String(case));
        let index = self.heap.alloc(Value::Number(index as f64));
        let mut record: crate::heap::Record = [("name".into(), label), ("index".into(), index)]
            .into_iter()
            .collect();
        if let Some(value) = value {
            record.order.push("value".into());
            record.insert("value".into(), value);
        }
        record.enum_name = Some(name);
        self.heap.alloc(Value::Record(record))
    }
    /// All textual output shares one formatter. Strings are raw at the
    /// top level and quoted within structured data, matching existing output.
    fn value_text(&self, value: Handle) -> Result<String> {
        self.inspect(value, false, &mut vec![])
    }
    fn inspect(&self, value: Handle, nested: bool, ancestors: &mut Vec<Handle>) -> Result<String> {
        match self.heap.get(value)? {
            Value::Unit => Ok("None".into()),
            Value::Number(n) => Ok(number_text(*n)),
            Value::Bool(b) => Ok(b.to_string()),
            Value::String(s) if nested => serde_json::to_string(s).map_err(|e| e.to_string()),
            Value::String(s) => Ok(s.clone()),
            Value::List(_) | Value::Record(_) if ancestors.contains(&value) => {
                Ok("[Circular]".into())
            }
            Value::List(_) | Value::Record(_) if ancestors.len() >= 24 => Ok("…".into()),
            Value::List(items) => {
                ancestors.push(value);
                let parts = items
                    .iter()
                    .map(|h| self.inspect(*h, true, ancestors))
                    .collect::<Result<Vec<_>>>()?;
                ancestors.pop();
                Ok(if parts.is_empty() {
                    "[]".into()
                } else {
                    format!("[ {} ]", parts.join(", "))
                })
            }
            Value::Record(record) => {
                ancestors.push(value);
                if record.enum_name.is_some() {
                    let name = record.get("name").ok_or("enum has no case name")?;
                    let Value::String(name) = self.heap.get(*name)? else {
                        return Err("enum case name is not text".into());
                    };
                    let text = match record.get("value") {
                        Some(payload) => {
                            format!("{name}({})", self.inspect(*payload, true, ancestors)?)
                        }
                        None => name.clone(),
                    };
                    ancestors.pop();
                    return Ok(text);
                }
                let mut parts = vec![];
                for name in &record.order {
                    // Receiver methods are runtime attachments, not data fields.
                    if record.struct_name.is_some() && name.starts_with('$') {
                        continue;
                    }
                    if let Some(h) = record.get(name) {
                        parts.push(format!("{name}: {}", self.inspect(*h, true, ancestors)?));
                    }
                }
                ancestors.pop();
                let fields = if parts.is_empty() {
                    "{}".into()
                } else {
                    format!("{{ {} }}", parts.join(", "))
                };
                Ok(match &record.struct_name {
                    Some(name) => format!("{name} {fields}"),
                    None => fields,
                })
            }
            _ => Err("value has no printable form".into()),
        }
    }
    /// Reads and writes use the same declared embed traversal. Own fields
    /// win; arbitrary nested records do not implicitly promote their fields.
    fn field_owner(&self, object: Handle, name: &str) -> Result<Option<Handle>> {
        let Value::Record(record) = self.heap.get(object)? else {
            return Ok(None);
        };
        if record.contains_key(name) {
            return Ok(Some(object));
        }
        if record.struct_name.is_none() {
            return Ok(None);
        }
        for embed in &record.embeds {
            if let Some(value) = record.get(embed)
                && let Some(owner) = self.field_owner(*value, name)?
            {
                return Ok(Some(owner));
            }
        }
        Ok(None)
    }
    /// Shared call setup for `Call` and `MethodCall`: build the next frame,
    /// or spawn a task when the closure is async.
    fn invoke(
        &mut self,
        function: usize,
        captures: Vec<Handle>,
        args: Vec<Handle>,
        slot_children: Option<Handle>,
    ) -> Result<Invocation> {
        let next = self.frame(function, captures, args, slot_children)?;
        if self.program.functions[function].asynchronous {
            Ok(Invocation::Task(self.spawn(Work::Code(vec![next]))))
        } else {
            Ok(Invocation::Frame(next))
        }
    }
    /// Call `callee` with `args` on behalf of the frame on top of `frames`:
    /// enter its frame, or push the spawned task's promise for an async one.
    fn call_value(
        &mut self,
        frames: &mut Vec<Frame>,
        callee: Handle,
        args: Vec<Handle>,
    ) -> Result<()> {
        let Value::Closure {
            function,
            captures,
            slot_children,
        } = self.heap.get(callee)?.clone()
        else {
            return Err("value is not callable".into());
        };
        match self.invoke(function, captures, args, slot_children)? {
            Invocation::Task(h) => frames.last_mut().ok_or("empty call stack")?.stack.push(h),
            Invocation::Frame(next) => {
                if frames.len() >= 1024 {
                    return Err("call stack limit exceeded".into());
                }
                frames.push(next);
            }
        }
        Ok(())
    }
    /// Read one component prop for the frame on top of `frames`. An attribute
    /// is a getter closure, run on every read so the child sees the parent's
    /// current state; nested children are a retained list; an absent optional
    /// prop reads as unit. With `then_call`, the prop's value is then called
    /// with those arguments (a callback prop used as `props.onSelect()`).
    fn read_prop(
        &mut self,
        frames: &mut Vec<Frame>,
        value: Option<Handle>,
        then_call: Option<Vec<Handle>>,
    ) -> Result<()> {
        let Some(value) = value else {
            let unit = self.heap.alloc(Value::Unit);
            frames
                .last_mut()
                .ok_or("empty call stack")?
                .stack
                .push(unit);
            return Ok(());
        };
        if let Value::Closure {
            function,
            captures,
            slot_children,
        } = self.heap.get(value)?.clone()
        {
            let mut getter = self.frame(function, captures, vec![], slot_children)?;
            getter.then_call = then_call;
            if frames.len() >= 1024 {
                return Err("call stack limit exceeded".into());
            }
            frames.push(getter);
            return Ok(());
        }
        match then_call {
            Some(args) => self.call_value(frames, value, args),
            None => {
                frames
                    .last_mut()
                    .ok_or("empty call stack")?
                    .stack
                    .push(value);
                Ok(())
            }
        }
    }
    fn list_mut(&mut self, frame: &mut Frame, kind: &ListMut) -> Result<()> {
        // Every variant pops its arguments (if any) then the receiver list,
        // mutates it in place so aliases observe the change, and pushes the
        // result the declared signature promises.
        let numeric = |frame: &mut Frame, heap: &Heap| -> Result<f64> {
            let h = pop(frame)?;
            let Value::Number(n) = heap.get(h)? else {
                return Err("list built-in index must be number".into());
            };
            Ok(*n)
        };
        let clamp = |n: f64, len: usize| -> usize {
            if !n.is_finite() {
                return len;
            }
            (n.max(0.) as usize).min(len)
        };
        match kind {
            ListMut::Push => {
                let value = pop(frame)?;
                let list = pop(frame)?;
                let Value::List(items) = self.heap.get_mut(list)? else {
                    return Err("push requires list".into());
                };
                items.push(value);
                let length = items.len();
                frame
                    .stack
                    .push(self.heap.alloc(Value::Number(length as f64)));
            }
            ListMut::Pop | ListMut::Shift => {
                let list = pop(frame)?;
                let Value::List(items) = self.heap.get_mut(list)? else {
                    return Err("pop/shift requires list".into());
                };
                let taken = match kind {
                    ListMut::Pop => items.pop(),
                    _ => (!items.is_empty()).then(|| items.remove(0)),
                };
                let value = match taken {
                    Some(h) => h,
                    None => self.heap.alloc(Value::Unit),
                };
                frame.stack.push(value);
            }
            ListMut::Unshift => {
                let value = pop(frame)?;
                let list = pop(frame)?;
                let Value::List(items) = self.heap.get_mut(list)? else {
                    return Err("unshift requires list".into());
                };
                items.insert(0, value);
                let length = items.len();
                frame
                    .stack
                    .push(self.heap.alloc(Value::Number(length as f64)));
            }
            ListMut::Splice => {
                let delete = numeric(frame, &self.heap)?;
                let start = numeric(frame, &self.heap)?;
                let list = pop(frame)?;
                let removed = {
                    let Value::List(items) = self.heap.get_mut(list)? else {
                        return Err("splice requires list".into());
                    };
                    let start = clamp(start, items.len());
                    let end = (start + clamp(delete, items.len())).min(items.len());
                    items.drain(start..end).collect::<Vec<_>>()
                };
                frame.stack.push(self.heap.alloc(Value::List(removed)));
            }
            ListMut::Sort => {
                let list = pop(frame)?;
                let mut items = match self.heap.get(list)? {
                    Value::List(items) => items.clone(),
                    _ => return Err("sort requires list".into()),
                };
                let mut keys = Vec::new();
                for h in &items {
                    keys.push(match self.heap.get(*h)? {
                        Value::Number(n) => (0, *n, String::new()),
                        Value::String(s) => (1, 0., s.clone()),
                        _ => return Err("sort requires a list of numbers or strings".into()),
                    });
                }
                let mut ranked: Vec<_> = keys.into_iter().zip(items.drain(..)).collect();
                ranked.sort_by(|a, b| {
                    a.0.0
                        .cmp(&b.0.0)
                        .then(
                            a.0.1
                                .partial_cmp(&b.0.1)
                                .unwrap_or(std::cmp::Ordering::Equal),
                        )
                        .then(a.0.2.cmp(&b.0.2))
                });
                items = ranked.into_iter().map(|(_, h)| h).collect();
                let Value::List(target) = self.heap.get_mut(list)? else {
                    return Err("sort requires list".into());
                };
                *target = items;
                frame.stack.push(list);
            }
            ListMut::Reverse => {
                let list = pop(frame)?;
                let Value::List(items) = self.heap.get_mut(list)? else {
                    return Err("reverse requires list".into());
                };
                items.reverse();
                frame.stack.push(list);
            }
            ListMut::Fill => {
                let start = numeric(frame, &self.heap)?;
                let value = pop(frame)?;
                let list = pop(frame)?;
                let Value::List(items) = self.heap.get_mut(list)? else {
                    return Err("fill requires list".into());
                };
                let start = clamp(start, items.len());
                for slot in items.iter_mut().skip(start) {
                    *slot = value;
                }
                frame.stack.push(list);
            }
            ListMut::CopyWithin => {
                let start = numeric(frame, &self.heap)?;
                let target = numeric(frame, &self.heap)?;
                let list = pop(frame)?;
                let Value::List(items) = self.heap.get_mut(list)? else {
                    return Err("copyWithin requires list".into());
                };
                let start = clamp(start, items.len());
                let target = clamp(target, items.len());
                for i in 0..(items.len() - start).min(items.len() - target) {
                    items[target + i] = items[start + i];
                }
                frame.stack.push(list);
            }
        }
        Ok(())
    }
}
fn pop(frame: &mut Frame) -> Result<Handle> {
    frame
        .stack
        .pop()
        .ok_or_else(|| "operand stack underflow".into())
}
/// JavaScript's ToInt32: truncate toward zero, then keep the low 32 bits.
fn int32(n: f64) -> i32 {
    (n.trunc() as i64) as i32
}
/// How `string(x)` turns a number into text; string+number concat uses the
/// same conversion, as the note-03 decision requires.
fn number_text(n: f64) -> String {
    format!("{n}")
}
fn arguments(frame: &mut Frame, count: usize) -> Result<Vec<Handle>> {
    // Do not reserve an untrusted bytecode operand's claimed size.
    let mut args = Vec::new();
    for _ in 0..count {
        args.push(pop(frame)?);
    }
    args.reverse();
    Ok(args)
}
