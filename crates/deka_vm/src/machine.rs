use crate::{
    heap::{Handle, Heap, Outcome, Read, Value},
    stack::Stack,
    *,
};
use std::{
    collections::BTreeMap,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
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
    Join(Join),
    Host {
        future: HostFuture,
        result: HostType,
        result_channel: bool,
        job: Option<HostJob>,
        ready: Arc<crate::turn::ReadyWake>,
    },
}
/// A join owns a snapshot of the input handles, never the caller's list.
/// Scans are bounded and park until a promise settles; idle joins do not spin.
struct Join {
    kind: PromiseJoin,
    inputs: Vec<Handle>,
    values: Vec<Option<Handle>>,
    remaining: usize,
    cursor: usize,
    created_epoch: u64,
    scan_epoch: u64,
    checked_epoch: Option<u64>,
    first: Option<(u64, usize, Outcome)>,
}
struct Task {
    promise: Handle,
    work: Work,
    waiting: Option<Handle>,
}
enum Step {
    Continue,
    Blocked(Handle),
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
    context: HostContext,
    pub(crate) heap: Heap,
    pins: Vec<Handle>,
    tasks: BTreeMap<u64, Task>,
    next_task: u64,
    root: Option<Handle>,
    instructions: u64,
    instruction_limit: Option<u64>,
    descriptors: BTreeMap<TypeDescriptor, Handle>,
    wake: Arc<crate::turn::ReadyWake>,
    turn_cursor: u64,
    completion_epoch: u64,
    completion_order: BTreeMap<Handle, u64>,
    #[cfg(feature = "ui")]
    events: std::collections::BTreeSet<u64>,
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
            context: HostContext::default(),
            heap: Heap::default(),
            pins: vec![],
            tasks: BTreeMap::new(),
            next_task: 0,
            root: None,
            instructions: 0,
            instruction_limit: None,
            descriptors: BTreeMap::new(),
            wake: Arc::new(crate::turn::ReadyWake::default()),
            turn_cursor: 0,
            completion_epoch: 0,
            completion_order: BTreeMap::new(),
            #[cfg(feature = "ui")]
            events: std::collections::BTreeSet::new(),
        };
        let frame = vm.frame(0, vec![], vec![], None)?;
        vm.root = Some(vm.spawn(Work::Code(vec![frame])));
        vm.wake.wake_by_ref();
        Ok(vm)
    }
    pub fn stats(&self) -> HeapStats {
        self.heap.stats()
    }
    pub fn instructions(&self) -> u64 {
        self.instructions
    }
    /// Optional embedding guard; normal execution has no lifetime quota.
    pub fn set_instruction_limit(&mut self, limit: u64) {
        self.instruction_limit = Some(limit);
    }
    pub fn pending_tasks(&self) -> usize {
        self.tasks.len()
    }
    pub fn cancel(&mut self) -> Result<()> {
        self.context.close();
        self.tasks.clear();
        self.completion_order.clear();
        #[cfg(feature = "ui")]
        self.events.clear();
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
        roots.extend(self.context.roots());
        for task in self.tasks.values() {
            roots.push(task.promise);
            if let Work::Join(join) = &task.work {
                roots.extend(&join.inputs);
                roots.extend(join.values.iter().flatten());
            }
            if let Work::Code(frames) = &task.work {
                for f in frames {
                    roots.extend(&f.locals);
                    roots.extend(f.stack.roots());
                    roots.extend(f.slot_children);
                    roots.extend(f.then_call.iter().flatten());
                }
            }
        }
        self.heap.collect(roots)?;
        self.completion_order
            .retain(|handle, _| matches!(self.heap.get(*handle), Ok(Value::Promise(Some(_)))));
        Ok(())
    }
    fn settle_promise(&mut self, promise: Handle, outcome: Result<Outcome>) -> Result<()> {
        self.heap.replace(promise, Value::Promise(Some(outcome)))?;
        self.completion_epoch += 1;
        self.completion_order.insert(promise, self.completion_epoch);
        Ok(())
    }
    fn join(&mut self, kind: PromiseJoin, list: Handle) -> Result<Handle> {
        let Value::List(inputs) = self.heap.get(list)? else {
            return Err("promise combinator requires a list".into());
        };
        let inputs = inputs.clone();
        let len = inputs.len();
        Ok(self.spawn(Work::Join(Join {
            kind,
            inputs,
            values: vec![None; len],
            remaining: len,
            cursor: 0,
            created_epoch: self.completion_epoch,
            scan_epoch: self.completion_epoch,
            checked_epoch: None,
            first: None,
        })))
    }
    fn poll_join(&mut self, join: &mut Join, remaining: &mut usize) -> Result<Option<Outcome>> {
        if join.cursor == 0 {
            join.scan_epoch = self.completion_epoch;
        }
        for _ in 0..256.min(*remaining) {
            *remaining -= 1;
            if join.cursor == join.inputs.len() {
                break;
            }
            let promise = join.inputs[join.cursor];
            match self.heap.get(promise)? {
                Value::Promise(Some(result)) => {
                    let outcome = result.clone()?;
                    if join.values[join.cursor].is_none()
                        && let Outcome::Value(value) = outcome
                    {
                        join.values[join.cursor] = Some(value);
                        join.remaining -= 1;
                    }
                    if join.kind == PromiseJoin::Race || matches!(outcome, Outcome::Thrown(_)) {
                        let order = *self
                            .completion_order
                            .get(&promise)
                            .ok_or("missing promise settlement order")?;
                        let order = if order <= join.created_epoch {
                            0
                        } else {
                            order
                        };
                        if join
                            .first
                            .as_ref()
                            .is_none_or(|(first, index, _)| (order, join.cursor) < (*first, *index))
                        {
                            join.first = Some((order, join.cursor, outcome));
                        }
                    }
                }
                Value::Promise(None) => {}
                _ => return Err("promise combinator requires promises".into()),
            }
            join.cursor += 1;
        }
        if join.cursor < join.inputs.len() {
            return Ok(None);
        }
        if let Some((order, _, _)) = &join.first {
            // A candidate newer than the start of this scan could have an
            // earlier rival that settled behind our cursor. Scan once more;
            // later unrelated completions cannot delay an established winner.
            if *order > join.scan_epoch {
                join.cursor = 0;
                join.checked_epoch = None;
                return Ok(None);
            }
        }
        if let Some((_, _, outcome)) = join.first.take() {
            return Ok(Some(outcome));
        }
        if join.kind == PromiseJoin::All && join.remaining == 0 {
            let values = join.values.iter().flatten().copied().collect();
            return Ok(Some(Outcome::Value(self.heap.alloc(Value::List(values)))));
        }
        join.cursor = 0;
        join.checked_epoch = Some(join.scan_epoch);
        Ok(None)
    }
    fn spawn(&mut self, work: Work) -> Handle {
        let promise = self.heap.alloc(Value::Promise(None));
        let id = self.next_task;
        self.next_task += 1;
        if let Work::Host { ready, .. } = &work {
            ready.wake_by_ref();
        }
        self.tasks.insert(
            id,
            Task {
                promise,
                work,
                waiting: None,
            },
        );
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
        self.to_host_inner(h, &mut Vec::new())
    }
    fn to_host_inner(&self, h: Handle, ancestors: &mut Vec<Handle>) -> Result<HostValue> {
        if ancestors.contains(&h) {
            return Err("cyclic value cannot cross the host boundary".into());
        }
        if ancestors.len() >= 64 {
            return Err("host value nesting limit exceeded".into());
        }
        ancestors.push(h);
        let value = match self.heap.get(h)? {
            Value::Unit => HostValue::Unit,
            Value::Number(n) => HostValue::Number(*n),
            Value::Bool(b) => HostValue::Bool(*b),
            Value::String(s) => HostValue::String(s.clone()),
            Value::Bytes(bytes) => HostValue::Bytes(bytes.clone()),
            Value::Host(handle) => HostValue::Handle(handle.clone()),
            Value::List(items) => HostValue::List(
                items
                    .iter()
                    .map(|h| self.to_host_inner(*h, ancestors))
                    .collect::<Result<_>>()?,
            ),
            Value::Record(record) if record.enum_name.as_deref() == Some("Option") => {
                let name = record.get("name").ok_or("Option has no case")?;
                match self.heap.get(*name)? {
                    Value::String(case) if case == "None" => HostValue::Option(None),
                    Value::String(case) if case == "Some" => {
                        HostValue::Option(Some(Box::new(self.to_host_inner(
                            *record.get("value").ok_or("Some has no payload")?,
                            ancestors,
                        )?)))
                    }
                    _ => return Err("invalid Option case".into()),
                }
            }
            // Enums cross through their declared Result channel, not record-shape guessing.
            Value::Record(record) if record.enum_name.is_none() => HostValue::Record(
                record
                    .fields
                    .iter()
                    .map(|(name, h)| Ok((name.clone(), self.to_host_inner(*h, ancestors)?)))
                    .collect::<Result<_>>()?,
            ),
            Value::Closure { .. } => HostValue::Callback(self.context.callback(h)),
            _ => return Err("unsupported host wire value".into()),
        };
        ancestors.pop();
        Ok(value)
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
        let payload = self.alloc_host_value(value)?;
        Ok(if result_channel {
            self.enum_value("Result".into(), "Ok".into(), 0, Some(payload))
        } else {
            payload
        })
    }
    pub(crate) fn alloc_host_value(&mut self, value: HostValue) -> Result<Handle> {
        self.alloc_host_value_inner(value, 0)
    }
    fn alloc_host_value_inner(&mut self, value: HostValue, depth: usize) -> Result<Handle> {
        if depth >= 64 {
            return Err("host value nesting limit exceeded".into());
        }
        let value = match value {
            HostValue::Option(value) => {
                return Ok(match value {
                    Some(value) => {
                        let payload = self.alloc_host_value_inner(*value, depth + 1)?;
                        self.enum_value("Option".into(), "Some".into(), 0, Some(payload))
                    }
                    None => self.enum_value("Option".into(), "None".into(), 1, None),
                });
            }
            HostValue::Bytes(bytes) => Value::Bytes(bytes),
            HostValue::Handle(handle) => Value::Host(handle),
            HostValue::List(items) => Value::List(
                items
                    .into_iter()
                    .map(|v| self.alloc_host_value_inner(v, depth + 1))
                    .collect::<Result<_>>()?,
            ),
            HostValue::Record(fields) => Value::Record(
                fields
                    .into_iter()
                    .map(|(name, v)| Ok((name, self.alloc_host_value_inner(v, depth + 1)?)))
                    .collect::<Result<_>>()?,
            ),
            HostValue::Callback(callback) => {
                if !self.context.owns(&callback) {
                    return Err("callback belongs to another VM".into());
                }
                self.heap.get(callback.handle())?;
                return Ok(callback.handle());
            }
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
        Ok(self.heap.alloc(value))
    }
    fn host_commands(&mut self, remaining: &mut usize) -> Result<bool> {
        let commands = self.context.take(*remaining);
        *remaining -= commands.len();
        let progressed = !commands.is_empty();
        for command in commands {
            match command {
                crate::callback::Command::Spawn { job, future } => {
                    if !job.is_cancelled() {
                        self.spawn(Work::Host {
                            future,
                            result: HostType::Unit,
                            result_channel: false,
                            job: Some(job),
                            ready: Arc::new(crate::turn::ReadyWake::default()),
                        });
                    }
                }
                crate::callback::Command::Cancel(id) => {
                    self.tasks.retain(|_, task| {
                        !matches!(
                            &task.work, Work::Host { job: Some(job), .. } if job.id() == id
                        )
                    });
                }
                crate::callback::Command::Call {
                    callback,
                    args,
                    job,
                } => {
                    if job.is_some_and(|job| job.is_cancelled()) {
                        continue;
                    }
                    if !self.context.owns(&callback) {
                        return Err("callback belongs to another VM".into());
                    }
                    let Value::Closure {
                        function,
                        captures,
                        slot_children,
                    } = self.heap.get(callback.handle())?.clone()
                    else {
                        return Err("host callback is not callable".into());
                    };
                    let args = args
                        .into_iter()
                        .map(|value| self.alloc_host_value(value))
                        .collect::<Result<Vec<_>>>()?;
                    let frame = self.frame(function, captures, args, slot_children)?;
                    self.spawn(Work::Code(vec![frame]));
                }
            }
        }
        Ok(progressed)
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
    /// Register the platform's wake without exposing its event-loop types.
    pub fn set_waker(&mut self, waker: &Waker) {
        self.wake.set_parent(waker);
        self.context.set_waker(&Waker::from(self.wake.clone()));
        if self.has_ready_work() {
            waker.wake_by_ref();
        }
    }
    pub fn waker(&self) -> Waker {
        self.wake.parent()
    }
    pub fn has_ready_work(&self) -> bool {
        self.root.is_some() && (self.wake.is_ready() || self.context.has_commands())
    }
    /// Drive persistent work, even after initialization/handlers have returned.
    /// This budget bounds one turn, independent of the lifetime of an app.
    pub fn run_turn(&mut self, cx: &mut Context<'_>, budget: usize) -> Result<Turn> {
        if budget == 0 {
            return Err("turn budget must be positive".into());
        }
        if self.root.is_none() {
            return Err("VM cancelled".into());
        }
        self.wake.set_parent(cx.waker());
        self.wake.clear();
        let waker = Waker::from(self.wake.clone());
        self.context.set_waker(&waker);
        let before = self.instructions;
        let result = self.drive(&mut Context::from_waker(&waker), budget, None, None);
        match result {
            Ok((progressed, deferred)) => {
                if ((progressed || deferred) && !self.tasks.is_empty())
                    || self.context.has_commands()
                {
                    self.wake.wake_by_ref();
                }
                Ok(Turn {
                    progressed,
                    instructions: self.instructions - before,
                })
            }
            Err(error) => {
                self.cancel()?;
                Err(error)
            }
        }
    }
    fn drive(
        &mut self,
        cx: &mut Context<'_>,
        budget: usize,
        only: Option<Handle>,
        instruction_limit: Option<u64>,
    ) -> Result<(bool, bool)> {
        let mut remaining = budget;
        let mut deferred = false;
        let mut progressed = if only.is_none() {
            self.host_commands(&mut remaining)?
        } else {
            false
        };
        if only.is_none() {
            for task in self.tasks.values() {
                if let Work::Host { ready, .. } = &task.work {
                    ready.set_parent(cx.waker());
                }
            }
        }
        let ids: Vec<_> = self
            .tasks
            .range(self.turn_cursor..)
            .chain(self.tasks.range(..self.turn_cursor))
            .filter(|(_, task)| {
                only.is_none_or(|root| task.promise == root)
                    && task.waiting.is_none_or(|promise| {
                        !matches!(self.heap.get(promise), Ok(Value::Promise(None)))
                    })
                    && match &task.work {
                        Work::Host { ready, .. } => ready.is_ready(),
                        Work::Code(_) => true,
                        Work::Join(join) => join.checked_epoch != Some(self.completion_epoch),
                    }
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if remaining == 0 {
                deferred = true;
                break;
            }
            let mut task = self.tasks.remove(&id).ok_or("missing scheduled task")?;
            task.waiting = None;
            let result = match &mut task.work {
                Work::Join(join) => {
                    let before = remaining;
                    let result = self.poll_join(join, &mut remaining)?;
                    progressed |= before != remaining;
                    result.map(Ok)
                }
                Work::Host {
                    future,
                    result,
                    result_channel,
                    ready,
                    ..
                } => {
                    remaining -= 1;
                    ready.clear();
                    let waker = Waker::from(ready.clone());
                    match future.as_mut().poll(&mut Context::from_waker(&waker)) {
                        Poll::Ready(v) => Some(
                            self.host_result(v, result.clone(), *result_channel)
                                .map(Outcome::Value),
                        ),
                        Poll::Pending => None,
                    }
                }
                Work::Code(frames) => {
                    let mut completion = None;
                    for _ in 0..256.min(remaining) {
                        remaining -= 1;
                        self.instructions += 1;
                        if instruction_limit.is_some_and(|limit| self.instructions > limit) {
                            return Err("instruction limit exceeded".into());
                        }
                        match self.step(frames) {
                            Ok(Step::Continue) => progressed = true,
                            Ok(Step::Blocked(promise)) => {
                                task.waiting = Some(promise);
                                break;
                            }
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
            if only.is_none() {
                self.turn_cursor = id.saturating_add(1);
            }
            if let Some(result) = result {
                let result = result?;
                #[cfg(feature = "ui")]
                if self.events.remove(&id)
                    && let Outcome::Thrown(value) = &result
                {
                    return Err(format!("uncaught Throw: {}", self.value_text(*value)?));
                }
                progressed = true;
                self.settle_promise(task.promise, Ok(result))?;
            } else {
                self.tasks.insert(id, task);
            }
        }
        if only.is_none() {
            progressed |= self.host_commands(&mut remaining)?;
        }
        self.collect()?;
        Ok((progressed, deferred))
    }
    fn root_result(&self) -> Poll<Result<Handle>> {
        let Some(root) = self.root else {
            return Poll::Ready(Err("VM cancelled".into()));
        };
        match self.heap.get(root) {
            Ok(Value::Promise(Some(Ok(Outcome::Value(h))))) => Poll::Ready(Ok(*h)),
            Ok(Value::Promise(Some(Ok(Outcome::Thrown(h))))) => {
                Poll::Ready(Err(format!("uncaught Throw: {}", self.value_text(*h)?)))
            }
            Ok(Value::Promise(Some(Err(e)))) => Poll::Ready(Err(e.clone())),
            Ok(Value::Promise(None)) => Poll::Pending,
            _ => Poll::Ready(Err("invalid entry promise".into())),
        }
    }
    fn poll_inner(&mut self, cx: &mut Context<'_>, drain_tasks: bool) -> Poll<Result<Handle>> {
        if self.root.is_none() {
            return Poll::Ready(Err("VM cancelled".into()));
        }
        self.wake.set_parent(cx.waker());
        self.wake.clear();
        let waker = Waker::from(self.wake.clone());
        self.context.set_waker(&waker);
        let (progressed, deferred) = self.drive(
            &mut Context::from_waker(&waker),
            4096,
            None,
            self.instruction_limit,
        )?;
        let root = self.root_result();
        if root.is_pending()
            || (drain_tasks
                && matches!(root, Poll::Ready(Ok(_)))
                && (!self.tasks.is_empty() || self.context.has_commands()))
        {
            if progressed || deferred || self.context.has_commands() {
                self.wake.wake_by_ref();
            }
            Poll::Pending
        } else {
            root
        }
    }
    #[cfg(feature = "ui")]
    pub(crate) fn finish_sync(&mut self) -> Result<Handle> {
        let root = self.root.ok_or("VM cancelled")?;
        let mut cx = Context::from_waker(Waker::noop());
        let child_start = self.next_task;
        let limit = self.instructions.saturating_add(10_000_000);
        loop {
            // Bindings evaluate only their own frame. Other tasks and host
            // commands remain owned by the persistent scheduler.
            let (progressed, _) = self.drive(&mut cx, usize::MAX, Some(root), Some(limit))?;
            if let Poll::Ready(result) = self.root_result() {
                if self.next_task > child_start || self.context.has_commands() {
                    self.wake.wake_by_ref();
                }
                return result;
            }
            if !progressed {
                return Err("UI bindings must complete synchronously".into());
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
        let Value::Closure {
            function,
            captures,
            slot_children,
        } = self.heap.get(closure)?.clone()
        else {
            return Err("UI handler is not a closure".into());
        };
        if self.program.functions[function].asynchronous {
            return Err("UI bindings must complete synchronously".into());
        }
        let frame = self.frame(function, captures, args, slot_children)?;
        self.root = Some(self.spawn(Work::Code(vec![frame])));
        self.finish_sync()
    }
    #[cfg(feature = "ui")]
    pub(crate) fn enqueue_event(&mut self, closure: Handle, args: Vec<Handle>) -> Result<()> {
        let Value::Closure {
            function,
            captures,
            slot_children,
        } = self.heap.get(closure)?.clone()
        else {
            return Err("UI handler is not a closure".into());
        };
        let frame = self.frame(function, captures, args, slot_children)?;
        self.events.insert(self.next_task);
        self.spawn(Work::Code(vec![frame]));
        self.wake.wake_by_ref();
        Ok(())
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
                self.heap.observe(frame.locals[i], Read::Cell)?;
                let Value::Cell(h) = self.heap.get(frame.locals[i])? else {
                    return Err("invalid local cell".into());
                };
                frame.stack.push(*h);
            }
            Op::LoadChecked { slot, message } => {
                self.heap.observe(frame.locals[slot], Read::Cell)?;
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
                    (a, b)
                        if matches!(op, Op::Add)
                            && (matches!(a, Value::Promise(_))
                                || matches!(b, Value::Promise(_))) =>
                    {
                        Value::String(promise_add_text(a)? + &promise_add_text(b)?)
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
            Op::JsonStringify(shape) => {
                let value = pop(frame)?;
                let text = crate::json::stringify(&self.heap, value, &shape)?;
                frame.stack.push(self.heap.alloc(Value::String(text)));
            }
            Op::JsonParse(shape) => {
                let factories = pop(frame)?;
                let text = pop(frame)?;
                let result = self.parse_json(text, &shape, factories)?;
                frame.stack.push(result);
            }
            Op::JsonParseResult(shape) => {
                let factories = pop(frame)?;
                let result = pop(frame)?;
                let Value::Record(record) = self.heap.get(result)? else {
                    return Err("JSON body read requires a Result".into());
                };
                if record.enum_name.as_deref() != Some("Result") {
                    return Err("JSON body read requires a nominal Result".into());
                }
                let case = *record.get("name").ok_or("JSON body Result has no case")?;
                let payload = *record
                    .get("value")
                    .ok_or("JSON body Result has no payload")?;
                let value = match self.heap.get(case)? {
                    Value::String(case) if case == "Err" => result,
                    Value::String(case) if case == "Ok" => {
                        self.parse_json(payload, &shape, factories)?
                    }
                    _ => return Err("invalid JSON body Result case".into()),
                };
                frame.stack.push(value);
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
                self.heap.observe(h, Read::Field("name".into()))?;
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
                let h = pop(frame)?;
                self.heap.observe(h, Read::Length)?;
                let matched =
                    matches!(self.heap.get(h)?, Value::List(items) if items.len() == length);
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
                self.heap.observe(receiver, Read::Field(name.clone()))?;
                self.heap
                    .observe(receiver, Read::Field(format!("${name}")))?;
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
                self.heap.volatile();
                let args = arguments(frame, argc)?
                    .into_iter()
                    .map(|h| self.to_host(h))
                    .collect::<Result<Vec<_>>>()?;
                let (reply, expected, asynchronous, result_channel) =
                    self.hosts.call(&operation, args, &self.context)?;
                let h = match reply {
                    HostReply::Pending(future) => self.spawn(Work::Host {
                        future,
                        job: None,
                        ready: Arc::new(crate::turn::ReadyWake::default()),
                        result: expected,
                        result_channel,
                    }),
                    HostReply::Ready(value) => {
                        let result = self.host_result(value, expected, result_channel);
                        if asynchronous {
                            let promise = self.heap.alloc(Value::Promise(None));
                            self.settle_promise(promise, result.map(Outcome::Value))?;
                            promise
                        } else {
                            result?
                        }
                    }
                };
                frame.stack.push(h);
            }
            Op::PromiseJoin(kind) => {
                let list = pop(frame)?;
                frame.stack.push(self.join(kind, list)?);
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
                        return Ok(Step::Blocked(promise));
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
                self.heap.observe(list, Read::Length)?;
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
                self.heap.observe(list, Read::Entity)?;
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
                self.heap.observe(source, Read::Entity)?;
                self.heap.observe(target, Read::Entity)?;
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
                self.heap.observe(source, Read::Entity)?;
                self.heap.observe(target, Read::Entity)?;
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
                self.heap.observe(
                    h,
                    if name == "length" && matches!(self.heap.get(h)?, Value::List(_)) {
                        Read::Length
                    } else {
                        Read::Field(name.clone())
                    },
                )?;
                if let Value::Props(fields) = self.heap.get(h)? {
                    let value = fields.get(&name).copied();
                    self.read_prop(frames, value, None)?;
                    return Ok(Step::Continue);
                }
                let value = match self.heap.get(h)? {
                    Value::List(items) if name == "length" => {
                        self.heap.alloc(Value::Number(items.len() as f64))
                    }
                    Value::Bytes(bytes) if name == "length" => {
                        self.heap.alloc(Value::Number(bytes.len() as f64))
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
                self.heap.observe(h, Read::Field(name.clone()))?;
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
                self.heap.observe(object, Read::Index(index))?;
                match self.heap.get(object)? {
                    Value::List(items) => {
                        frame
                            .stack
                            .push(*items.get(index).ok_or("index out of bounds")?);
                    }
                    Value::Bytes(bytes) => {
                        let byte = *bytes.get(index).ok_or("index out of bounds")?;
                        frame
                            .stack
                            .push(self.heap.alloc(Value::Number(byte as f64)));
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
            Value::Bytes(_) => TypeDescriptor::new("bytes", "bytes"),
            Value::Host(handle) => TypeDescriptor::new("opaque", handle.name()),
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
    fn parse_json(
        &mut self,
        text: Handle,
        shape: &crate::JsonShape,
        factories: Handle,
    ) -> Result<Handle> {
        let Value::String(text) = self.heap.get(text)? else {
            return Err("JSON.parse requires string input".into());
        };
        let text = text.clone();
        let (case, index, value) = match crate::json::parse(&mut self.heap, &text, shape, factories)
        {
            Ok(value) => ("Ok", 0, value),
            Err(error) => ("Err", 1, self.heap.alloc(Value::String(error))),
        };
        Ok(self.enum_value("Result".into(), case.into(), index, Some(value)))
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
        self.heap.alloc_enum(name, case, index, value)
    }
    /// All textual output shares one formatter. Strings are raw at the
    /// top level and quoted within structured data, matching existing output.
    fn value_text(&self, value: Handle) -> Result<String> {
        self.inspect(value, false, &mut vec![])
    }
    fn inspect(&self, value: Handle, nested: bool, ancestors: &mut Vec<Handle>) -> Result<String> {
        self.heap.observe(value, Read::Entity)?;
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
        self.heap.observe(object, Read::Field(name.into()))?;
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
            self.heap.observe(object, Read::Field(embed.clone()))?;
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
// Only the checker's existing Promise + scalar rule uses this formatter.
// It never awaits a Promise or widens general printing/string conversion.
fn promise_add_text(value: Value) -> Result<String> {
    match value {
        Value::Promise(_) => Ok("[object Promise]".into()),
        Value::String(text) => Ok(text),
        Value::Number(number) => Ok(number_text(number)),
        Value::Bool(value) => Ok(value.to_string()),
        _ => Err("invalid arithmetic operands".into()),
    }
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

impl Drop for Vm {
    fn drop(&mut self) {
        self.context.close();
    }
}

#[cfg(all(test, feature = "compiler"))]
mod host_wire_tests {
    use super::*;
    #[test]
    fn wire_conversion_rejects_cycles_and_depth_but_allows_shared_children() {
        let hosts = Hosts::default();
        let program = compiler::compile("fn main() {}", &hosts).unwrap();
        let mut vm = Vm::new(program, hosts).unwrap();
        let a = vm.heap.alloc(Value::Unit);
        vm.heap.replace(a, Value::List(vec![a])).unwrap();
        assert_eq!(
            vm.to_host(a).unwrap_err(),
            "cyclic value cannot cross the host boundary"
        );
        let leaf = vm.heap.alloc(Value::Number(7.));
        let shared = vm.heap.alloc(Value::List(vec![leaf, leaf]));
        assert_eq!(
            vm.to_host(shared).unwrap(),
            HostValue::List(vec![HostValue::Number(7.); 2])
        );
        let mut deep = leaf;
        for _ in 0..64 {
            deep = vm.heap.alloc(Value::List(vec![deep]));
        }
        assert_eq!(
            vm.to_host(deep).unwrap_err(),
            "host value nesting limit exceeded"
        );
        let mut host = HostValue::Unit;
        for _ in 0..64 {
            host = HostValue::List(vec![host]);
        }
        assert_eq!(
            vm.alloc_host_value(host).unwrap_err(),
            "host value nesting limit exceeded"
        );
        vm.cancel().unwrap();
        assert_eq!(vm.stats().live, 0);
    }
}

#[cfg(test)]
mod promise_tests {
    use super::*;
    fn machine() -> Vm {
        Vm::new(
            Program {
                version: 1,
                functions: vec![Function {
                    name: "main".into(),
                    parameters: 0,
                    captures: 0,
                    locals: 0,
                    asynchronous: false,
                    code: vec![Op::Const(Literal::Unit), Op::Return],
                }],
            },
            Hosts::default(),
        )
        .unwrap()
    }
    fn take_join(vm: &mut Vm, inputs: Vec<Handle>, kind: PromiseJoin) -> Join {
        let list = vm.heap.alloc(Value::List(inputs));
        vm.join(kind, list).unwrap();
        let (_, task) = vm.tasks.pop_last().unwrap();
        let Work::Join(join) = task.work else {
            panic!("expected join")
        };
        join
    }
    #[test]
    fn race_and_all_choose_the_first_settlement_even_behind_a_partial_scan() {
        for kind in PromiseJoin::ALL {
            let mut vm = machine();
            let inputs: Vec<_> = (0..300)
                .map(|_| vm.heap.alloc(Value::Promise(None)))
                .collect();
            let mut join = take_join(&mut vm, inputs.clone(), kind);
            assert!(vm.poll_join(&mut join, &mut 256).unwrap().is_none());
            assert_eq!(join.cursor, 256);
            let first = vm.heap.alloc(Value::Number(111.));
            let later = vm.heap.alloc(Value::Number(999.));
            let outcome = |v| {
                if kind == PromiseJoin::All {
                    Outcome::Thrown(v)
                } else {
                    Outcome::Value(v)
                }
            };
            vm.settle_promise(inputs[0], Ok(outcome(first))).unwrap();
            vm.settle_promise(inputs[299], Ok(outcome(later))).unwrap();
            assert!(vm.poll_join(&mut join, &mut 256).unwrap().is_none());
            assert!(vm.poll_join(&mut join, &mut 256).unwrap().is_none());
            let result = vm.poll_join(&mut join, &mut 256).unwrap().unwrap();
            match result {
                Outcome::Value(h) | Outcome::Thrown(h) => assert_eq!(h, first),
            }
        }
    }
    #[test]
    fn already_settled_race_inputs_use_input_order_not_historical_order() {
        let mut vm = machine();
        let inputs: Vec<_> = (0..2)
            .map(|_| vm.heap.alloc(Value::Promise(None)))
            .collect();
        let first = vm.heap.alloc(Value::Number(1.));
        let second = vm.heap.alloc(Value::Number(2.));
        vm.settle_promise(inputs[1], Ok(Outcome::Value(second)))
            .unwrap();
        vm.settle_promise(inputs[0], Ok(Outcome::Value(first)))
            .unwrap();
        let mut join = take_join(&mut vm, inputs, PromiseJoin::Race);
        let Outcome::Value(h) = vm.poll_join(&mut join, &mut 256).unwrap().unwrap() else {
            panic!("unexpected Throw")
        };
        assert_eq!(h, first);
    }
    #[test]
    fn joins_obey_scan_budgets_and_settlement_metadata_does_not_root_dead_values() {
        let mut vm = machine();
        let inputs: Vec<_> = (0..1000)
            .map(|_| vm.heap.alloc(Value::Promise(None)))
            .collect();
        let value = vm.heap.alloc(Value::Number(1.));
        for promise in &inputs {
            vm.settle_promise(*promise, Ok(Outcome::Value(value)))
                .unwrap();
        }
        let mut join = take_join(&mut vm, inputs, PromiseJoin::All);
        let mut remaining = 8;
        assert!(vm.poll_join(&mut join, &mut remaining).unwrap().is_none());
        assert_eq!(join.cursor, 8);
        assert_eq!(remaining, 0);
        assert_eq!(vm.completion_order.len(), 1000);
        drop(join);
        vm.collect().unwrap();
        assert!(vm.root.is_some());
        assert!(vm.completion_order.is_empty());
        assert_eq!(vm.stats().live, 1);
    }
}
