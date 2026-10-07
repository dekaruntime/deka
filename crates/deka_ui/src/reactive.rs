//! Scoped, single-threaded reactive state for Rust UI applications.
use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fmt,
    marker::PhantomData,
    ops::{AddAssign, SubAssign},
    rc::{Rc, Weak},
};

thread_local! {
    static NEXT_SCOPE: Cell<u64> = const { Cell::new(0) };
    static SCOPES: RefCell<BTreeMap<u64, Weak<Core>>> = const { RefCell::new(BTreeMap::new()) };
    static CURRENT: RefCell<Option<Rc<Core>>> = const { RefCell::new(None) };
}

/// Owns signals and reactions. Dropping the last scope releases their values and
/// closures; copied handles do not extend the scope's lifetime.
#[derive(Clone)]
pub struct Scope(Rc<Core>);
impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}
impl Scope {
    pub fn new() -> Self {
        let id = NEXT_SCOPE.with(|next| {
            let id = next.get();
            next.set(
                id.checked_add(1)
                    .expect("reactive scope identity exhausted"),
            );
            id
        });
        let core = Rc::new(Core {
            id,
            ..Core::default()
        });
        SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            scopes.retain(|_, scope| scope.strong_count() != 0);
            scopes.insert(id, Rc::downgrade(&core));
        });
        Self(core)
    }
    /// Construct reactive state or run work in this scope. Nested scopes restore
    /// the previous context, including when the closure unwinds.
    pub fn run<R>(&self, run: impl FnOnce() -> R) -> R {
        let previous = CURRENT.with(|current| current.replace(Some(self.0.clone())));
        let _context = Context(previous);
        run()
    }
    /// Coalesce all writes into one reaction pass. Nested batches coalesce too.
    pub fn batch<R>(&self, run: impl FnOnce() -> R) -> R {
        self.run(|| {
            self.0.depth.set(self.0.depth.get() + 1);
            let _batch = Batch(self.0.clone());
            run()
        })
    }
    /// Resume pending reactions after a caught panic or an explicit batch.
    pub fn flush(&self) {
        self.0.flush();
    }
}
struct Context(Option<Rc<Core>>);
impl Drop for Context {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.0.take());
        });
    }
}
struct Batch(Rc<Core>);
impl Drop for Batch {
    fn drop(&mut self) {
        self.0.depth.set(self.0.depth.get() - 1);
        if !std::thread::panicking() {
            self.0.flush();
        }
    }
}
fn current() -> Rc<Core> {
    CURRENT
        .with(|scope| scope.borrow().clone())
        .expect("create reactive state inside Scope::run")
}
fn resolve(id: u64) -> Rc<Core> {
    SCOPES
        .with(|scopes| scopes.borrow().get(&id).and_then(Weak::upgrade))
        .expect("reactive handle's scope has been dropped")
}

/// A Copy handle to state. Reads in a reaction subscribe that reaction; writes
/// schedule only subscribers. Handles stay on their owning UI thread.
pub struct Signal<T: 'static> {
    scope: u64,
    slot: usize,
    marker: PhantomData<fn() -> T>,
    thread: PhantomData<Rc<()>>,
}
impl<T> Copy for Signal<T> {}
impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}
struct Value<T>(RefCell<Option<T>>);
fn allocate<T: 'static>(value: Option<T>) -> Signal<T> {
    let core = current();
    let mut values = core.values.borrow_mut();
    let slot = values.len();
    values.push(Rc::new(Value(RefCell::new(value))));
    core.subscribers.borrow_mut().push(BTreeSet::new());
    core.revisions.borrow_mut().push(0);
    Signal {
        scope: core.id,
        slot,
        marker: PhantomData,
        thread: PhantomData,
    }
}
/// Allocate a signal in the current scope.
pub fn signal<T: 'static>(value: T) -> Signal<T> {
    allocate(Some(value))
}
impl<T: 'static> Signal<T> {
    fn value(self, core: &Core) -> Rc<Value<T>> {
        core.values.borrow()[self.slot]
            .clone()
            .downcast::<Value<T>>()
            .unwrap_or_else(|_| panic!("reactive signal type mismatch"))
    }
    pub fn get(self) -> T
    where
        T: Clone,
    {
        let core = resolve(self.scope);
        CURRENT.with(|active| {
            if let Some(active) = active.borrow().as_ref() {
                assert!(
                    active.id == core.id || active.reads.borrow().is_empty(),
                    "a reaction cannot observe signals from another scope"
                );
            }
        });
        if let Some(reads) = core.reads.borrow_mut().last_mut() {
            reads
                .entry(self.slot)
                .or_insert(core.revisions.borrow()[self.slot]);
        }
        self.value(&core)
            .0
            .borrow()
            .as_ref()
            .expect("derived value not initialized")
            .clone()
    }
    /// Explicit writes notify even for equal values. Derived values suppress
    /// unchanged results, and retained property bindings compare authored values.
    pub fn set(self, value: T) {
        let core = resolve(self.scope);
        self.value(&core).0.replace(Some(value));
        core.changed(self.slot);
    }
    pub fn update<R>(self, update: impl FnOnce(&mut T) -> R) -> R {
        let core = resolve(self.scope);
        let value = self.value(&core);
        // Notify after releasing the value's borrow, including a partial write
        // followed by a panic. Unwinding queues work without executing callbacks.
        let _write = Write {
            core,
            slot: self.slot,
        };
        let mut value = value.0.borrow_mut();
        update(value.as_mut().expect("derived value not initialized"))
    }
}
struct Write {
    core: Rc<Core>,
    slot: usize,
}
impl Drop for Write {
    fn drop(&mut self) {
        self.core.changed(self.slot);
    }
}
impl Signal<bool> {
    pub fn toggle(self) {
        self.update(|value| *value = !*value);
    }
}
impl<T: AddAssign<T> + 'static> AddAssign<T> for Signal<T> {
    fn add_assign(&mut self, rhs: T) {
        self.update(|value| *value += rhs);
    }
}
impl<T: SubAssign<T> + 'static> SubAssign<T> for Signal<T> {
    fn sub_assign(&mut self, rhs: T) {
        self.update(|value| *value -= rhs);
    }
}
impl<T: Clone + fmt::Display> fmt::Display for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

/// Read-only computed state, cached between changes to its observed inputs.
pub struct Derived<T: 'static>(Signal<T>);
impl<T> Copy for Derived<T> {}
impl<T> Clone for Derived<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Clone> Derived<T> {
    pub fn get(self) -> T {
        self.0.get()
    }
}
impl<T: Clone + fmt::Display> fmt::Display for Derived<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}
/// Compute immediately, then recompute when its dynamically observed inputs
/// change. Derived reactions settle before effects, avoiding diamond glitches.
pub fn derived<T: PartialEq + 'static>(mut compute: impl FnMut() -> T + 'static) -> Derived<T> {
    let output = allocate(None);
    observe(Kind::Derived, move || {
        let next = compute();
        let core = resolve(output.scope);
        let value = output.value(&core);
        let changed = value.0.borrow().as_ref() != Some(&next);
        if changed {
            value.0.replace(Some(next));
            core.changed(output.slot);
        }
    });
    Derived(output)
}

/// A reaction registration. The scope owns its closure; dropping this Copy
/// handle leaves it running. Call dispose to explicitly unsubscribe it.
#[derive(Clone, Copy)]
pub struct Effect {
    scope: u64,
    slot: usize,
    marker: PhantomData<Rc<()>>,
}
impl Effect {
    pub fn dispose(self) {
        let core = resolve(self.scope);
        let observer = core.observers.borrow_mut()[self.slot].take();
        if let Some(observer) = observer {
            for dependency in observer.dependencies {
                core.subscribers.borrow_mut()[dependency].remove(&self.slot);
            }
        }
        core.dirty.borrow_mut().remove(&self.slot);
    }
}
/// Run now and after observed signals change. Conditional reads replace the
/// dependency set on every execution; unrelated signals do no reaction work.
pub fn effect(run: impl FnMut() + 'static) -> Effect {
    observe(Kind::Effect, run)
}
/// Batch writes in the current scope, as an event dispatcher does per event.
pub fn batch<R>(run: impl FnOnce() -> R) -> R {
    Scope(current()).batch(run)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Derived,
    Effect,
}
type Callback = Rc<RefCell<Box<dyn FnMut()>>>;
struct Observer {
    callback: Callback,
    dependencies: BTreeSet<usize>,
    kind: Kind,
}
#[derive(Default)]
struct Core {
    id: u64,
    values: RefCell<Vec<Rc<dyn Any>>>,
    observers: RefCell<Vec<Option<Observer>>>,
    subscribers: RefCell<Vec<BTreeSet<usize>>>,
    reads: RefCell<Vec<BTreeMap<usize, u64>>>,
    revisions: RefCell<Vec<u64>>,
    dirty: RefCell<BTreeSet<usize>>,
    depth: Cell<usize>,
    flushing: Cell<bool>,
}
fn observe(kind: Kind, run: impl FnMut() + 'static) -> Effect {
    let core = current();
    let slot = core.observers.borrow().len();
    core.observers.borrow_mut().push(Some(Observer {
        callback: Rc::new(RefCell::new(Box::new(run))),
        dependencies: BTreeSet::new(),
        kind,
    }));
    // Hold the batch boundary while installing nested reactions. In particular,
    // a derived value must finish its read capture before its dependents run.
    Scope(core.clone()).batch(|| core.evaluate(slot));
    Effect {
        scope: core.id,
        slot,
        marker: PhantomData,
    }
}
impl Core {
    fn changed(self: &Rc<Self>, slot: usize) {
        let mut revisions = self.revisions.borrow_mut();
        revisions[slot] = revisions[slot]
            .checked_add(1)
            .expect("signal revision exhausted");
        drop(revisions);
        self.dirty
            .borrow_mut()
            .extend(self.subscribers.borrow()[slot].iter().copied());
        if !std::thread::panicking() {
            self.flush();
        }
    }
    fn evaluate(self: &Rc<Self>, slot: usize) {
        let callback = self.observers.borrow()[slot]
            .as_ref()
            .map(|o| o.callback.clone());
        let Some(callback) = callback else {
            return;
        };
        Scope(self.clone()).run(|| {
            self.reads.borrow_mut().push(BTreeMap::new());
            let _capture = Capture {
                core: self.clone(),
                slot,
            };
            callback.borrow_mut()();
        });
    }
    fn flush(self: &Rc<Self>) {
        if self.depth.get() != 0 || self.flushing.replace(true) {
            return;
        }
        let _flushing = Flushing(self.clone());
        let mut turns = 0;
        loop {
            let next = {
                let dirty = self.dirty.borrow();
                let observers = self.observers.borrow();
                dirty
                    .iter()
                    .find(|slot| {
                        observers[**slot]
                            .as_ref()
                            .is_some_and(|o| o.kind == Kind::Derived)
                    })
                    .or_else(|| dirty.first())
                    .copied()
            };
            let Some(slot) = next else {
                break;
            };
            turns += 1;
            assert!(
                turns <= 10_000,
                "reactive cycle did not settle within 10000 reactions"
            );
            self.dirty.borrow_mut().remove(&slot);
            self.evaluate(slot);
        }
    }
}
struct Flushing(Rc<Core>);
impl Drop for Flushing {
    fn drop(&mut self) {
        self.0.flushing.set(false);
    }
}
struct Capture {
    core: Rc<Core>,
    slot: usize,
}
impl Drop for Capture {
    fn drop(&mut self) {
        let reads = self
            .core
            .reads
            .borrow_mut()
            .pop()
            .expect("reaction read capture");
        let dependencies: BTreeSet<_> = reads.keys().copied().collect();
        if let Some(observer) = self.core.observers.borrow_mut()[self.slot].as_mut() {
            let mut subscribers = self.core.subscribers.borrow_mut();
            for old in observer.dependencies.difference(&dependencies) {
                subscribers[*old].remove(&self.slot);
            }
            for new in dependencies.difference(&observer.dependencies) {
                subscribers[*new].insert(self.slot);
            }
            observer.dependencies = dependencies;
            if reads
                .iter()
                .any(|(slot, revision)| self.core.revisions.borrow()[*slot] != *revision)
            {
                self.core.dirty.borrow_mut().insert(self.slot);
            }
        }
    }
}
