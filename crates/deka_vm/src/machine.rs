use crate::{
    heap::{Handle, Heap, Value},
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
}
enum Work {
    Code(Vec<Frame>),
    Host {
        future: HostFuture,
        result: HostType,
    },
}
struct Task {
    promise: Handle,
    work: Work,
}
enum Step {
    Continue,
    Blocked,
    Complete(Handle),
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
        };
        let frame = vm.frame(0, vec![], vec![])?;
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
        self.root = None;
        self.heap.collect([])
    }
    pub fn collect(&mut self) -> Result<()> {
        let mut roots: Vec<_> = self
            .root
            .into_iter()
            .chain(self.pins.iter().copied())
            .collect();
        for task in self.tasks.values() {
            roots.push(task.promise);
            if let Work::Code(frames) = &task.work {
                for f in frames {
                    roots.extend(&f.locals);
                    roots.extend(f.stack.roots());
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
    ) -> Result<Frame> {
        let f = self
            .program
            .functions
            .get(function)
            .ok_or("unknown function")?;
        if args.len() != f.parameters || captures.len() != f.captures {
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
    fn host_result(&mut self, value: Result<HostValue>, expected: HostType) -> Result<Handle> {
        let value = value?;
        if !expected.accepts(&value) {
            return Err("host returned the wrong result type".into());
        }
        Ok(self.alloc_host_value(value))
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
    /// Poll all runnable tasks in bounded instruction slices. Host futures use the caller's waker.
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<HostValue>> {
        match self.poll_inner(cx) {
            Poll::Ready(result) => {
                let result = result.and_then(|h| self.to_host(h));
                let _ = self.cancel();
                Poll::Ready(result)
            }
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_inner(&mut self, cx: &mut Context<'_>) -> Poll<Result<Handle>> {
        let Some(root) = self.root else {
            return Poll::Ready(Err("VM cancelled".into()));
        };
        let mut progressed = false;
        let ids: Vec<_> = self.tasks.keys().copied().collect();
        for id in ids {
            let mut task = self.tasks.remove(&id).unwrap();
            let result = match &mut task.work {
                Work::Host { future, result } => match future.as_mut().poll(cx) {
                    Poll::Ready(v) => Some(self.host_result(v, *result)),
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
            Ok(Value::Promise(Some(Ok(h)))) => Poll::Ready(Ok(*h)),
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
            match self.poll_inner(&mut cx) {
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
        let Value::Closure { function, captures } = self.heap.get(closure)?.clone() else {
            return Err("UI handler is not a closure".into());
        };
        if self.program.functions[function].asynchronous {
            return Err("async UI callbacks are not supported yet".into());
        }
        let frame = self.frame(function, captures, args)?;
        self.root = Some(self.spawn(Work::Code(vec![frame])));
        self.finish_sync()
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
            Op::ToString => {
                let h = pop(frame)?;
                let value = match self.heap.get(h)? {
                    Value::String(_) => h,
                    Value::Number(n) => {
                        let text = number_text(*n);
                        self.heap.alloc(Value::String(text))
                    }
                    Value::Bool(b) => self.heap.alloc(Value::String(b.to_string())),
                    _ => return Err("string() only accepts number, bool or string".into()),
                };
                frame.stack.push(value);
            }
            Op::ToNumber => {
                let h = pop(frame)?;
                let value = match self.heap.get(h)? {
                    Value::Number(_) => h,
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
            Op::Closure { function, captures } => {
                let captures = captures.into_iter().map(|i| frame.locals[i]).collect();
                frame
                    .stack
                    .push(self.heap.alloc(Value::Closure { function, captures }));
            }
            Op::Call(argc) => {
                let args = arguments(frame, argc)?;
                let Value::Closure { function, captures } = self.heap.get(pop(frame)?)?.clone()
                else {
                    return Err("value is not callable".into());
                };
                let next = self.frame(function, captures, args)?;
                if self.program.functions[function].asynchronous {
                    frame.stack.push(self.spawn(Work::Code(vec![next])));
                } else {
                    if frames.len() >= 1024 {
                        return Err("call stack limit exceeded".into());
                    }
                    frames.push(next);
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
                let (reply, expected, asynchronous) = self.hosts.call(&operation, args)?;
                let h = match reply {
                    HostReply::Pending(future) => self.spawn(Work::Host {
                        future,
                        result: expected,
                    }),
                    HostReply::Ready(value) => {
                        let result = self.host_result(value, expected);
                        if asynchronous {
                            self.heap.alloc(Value::Promise(Some(result)))
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
                    Value::Promise(Some(result)) => frame.stack.push(result.clone()?),
                    Value::Promise(None) => {
                        frame.stack.push(promise);
                        frame.ip -= 1;
                        return Ok(Step::Blocked);
                    }
                    _ => return Err("await requires a promise".into()),
                }
            }
            Op::Return => {
                let result = pop(frame)?;
                frames.pop();
                if let Some(parent) = frames.last_mut() {
                    parent.stack.push(result);
                } else {
                    return Ok(Step::Complete(result));
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
            Op::Record(names) => {
                let items = arguments(frame, names.len())?;
                frame.stack.push(
                    self.heap
                        .alloc(Value::Record(names.into_iter().zip(items).collect())),
                );
            }
            Op::Field(name) => {
                let h = pop(frame)?;
                let value = match self.heap.get(h)? {
                    Value::List(items) if name == "length" => {
                        self.heap.alloc(Value::Number(items.len() as f64))
                    }
                    Value::String(text) if name == "length" => {
                        self.heap.alloc(Value::Number(text.chars().count() as f64))
                    }
                    Value::Record(fields) => *fields.get(&name).ok_or("missing field")?,
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
        }
        Ok(Step::Continue)
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
