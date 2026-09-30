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
    heap: Heap,
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
        self.root = None;
        self.heap.collect([])
    }
    pub fn collect(&mut self) -> Result<()> {
        let mut roots: Vec<_> = self.root.into_iter().collect();
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
    fn to_host(&self, h: Handle) -> Result<HostValue> {
        Ok(match self.heap.get(h)? {
            Value::Unit => HostValue::Unit,
            Value::Number(n) => HostValue::Number(*n),
            Value::Bool(b) => HostValue::Bool(*b),
            Value::String(s) => HostValue::String(s.clone()),
            _ => return Err("host wire supports scalar values only in this experiment".into()),
        })
    }
    fn host_result(&mut self, value: Result<HostValue>, expected: HostType) -> Result<Handle> {
        let value = value?;
        if !expected.accepts(&value) {
            return Err("host returned the wrong result type".into());
        }
        Ok(self.heap.alloc(match value {
            HostValue::Unit => Value::Unit,
            HostValue::Number(n) => Value::Number(n),
            HostValue::Bool(b) => Value::Bool(b),
            HostValue::String(s) => Value::String(s),
        }))
    }
    pub async fn run(&mut self) -> Result<HostValue> {
        std::future::poll_fn(|cx| self.poll(cx)).await
    }
    /// Poll all runnable tasks in bounded instruction slices. Host futures use the caller's waker.
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<HostValue>> {
        let outcome = self.poll_inner(cx);
        if matches!(outcome, Poll::Ready(Err(_))) {
            let _ = self.cancel();
        }
        outcome
    }
    fn poll_inner(&mut self, cx: &mut Context<'_>) -> Poll<Result<HostValue>> {
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
            Ok(Value::Promise(Some(Ok(h)))) => {
                let value = self.to_host(*h);
                // Main owns this experiment's task scope: finish cancels unawaited work.
                let _ = self.cancel();
                Poll::Ready(value)
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
            Op::Store(i) => {
                let h = pop(frame)?;
                self.heap.replace(frame.locals[i], Value::Cell(h))?;
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Less | Op::Equal => {
                let b = self.heap.get(pop(frame)?)?.clone();
                let a = self.heap.get(pop(frame)?)?.clone();
                let value = match (a, b) {
                    (Value::Number(a), Value::Number(b)) => match op {
                        Op::Add => Value::Number(a + b),
                        Op::Sub => Value::Number(a - b),
                        Op::Mul => Value::Number(a * b),
                        Op::Div => Value::Number(a / b),
                        Op::Less => Value::Bool(a < b),
                        _ => Value::Bool(a == b),
                    },
                    (Value::String(a), Value::String(b)) => match op {
                        Op::Add => Value::String(a + &b),
                        Op::Equal => Value::Bool(a == b),
                        _ => return Err("unsupported string operation".into()),
                    },
                    (Value::Bool(a), Value::Bool(b)) if matches!(op, Op::Equal) => {
                        Value::Bool(a == b)
                    }
                    _ => return Err("invalid arithmetic operands".into()),
                };
                frame.stack.push(self.heap.alloc(value));
            }
            Op::Jump(ip) => frame.ip = ip,
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
            Op::Record(names) => {
                let items = arguments(frame, names.len())?;
                frame.stack.push(
                    self.heap
                        .alloc(Value::Record(names.into_iter().zip(items).collect())),
                );
            }
            Op::Field(name) => {
                let h = pop(frame)?;
                let Value::Record(fields) = self.heap.get(h)? else {
                    return Err("field access needs a record".into());
                };
                frame.stack.push(*fields.get(&name).ok_or("missing field")?);
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
                let Value::List(items) = self.heap.get(object)? else {
                    return Err("index requires a list".into());
                };
                frame
                    .stack
                    .push(*items.get(*index as usize).ok_or("index out of bounds")?);
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
fn arguments(frame: &mut Frame, count: usize) -> Result<Vec<Handle>> {
    // Do not reserve an untrusted bytecode operand's claimed size.
    let mut args = Vec::new();
    for _ in 0..count {
        args.push(pop(frame)?);
    }
    args.reverse();
    Ok(args)
}
