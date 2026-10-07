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
    pub(crate) fn id(&self) -> u64 {
        self.0.id
    }
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
    pub fn on_error(&self, sink: impl Fn(ReactiveError) + 'static) {
        self.0.error_sink.replace(Some(Rc::new(sink)));
    }
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
pub(crate) fn current_scope_id() -> Option<u64> {
    CURRENT.with(|scope| scope.borrow().as_ref().map(|core| core.id))
}
fn current() -> Rc<Core> {
    CURRENT
        .with(|scope| scope.borrow().clone())
        .expect("create reactive state inside Scope::run")
}
fn resolve(id: u64) -> Result<Rc<Core>, ReactiveError> {
    SCOPES
        .with(|scopes| scopes.borrow().get(&id).and_then(Weak::upgrade))
        .ok_or(ReactiveError::DroppedScope)
}

/// Operational failures from a stale handle or a conflicting reactive read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactiveError {
    DroppedScope,
    DisposedSignal,
    CrossScopeRead,
    Uninitialized,
    BorrowedValue,
}
impl fmt::Display for ReactiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DroppedScope => "reactive scope has been dropped",
            Self::DisposedSignal => "reactive signal has been disposed",
            Self::CrossScopeRead => "a reaction cannot observe another scope",
            Self::Uninitialized => "derived value is not initialized",
            Self::BorrowedValue => "reactive value is already borrowed",
        })
    }
}
impl std::error::Error for ReactiveError {}
fn report(scope: u64, error: ReactiveError) {
    if let Some(core) = CURRENT
        .with(|current| current.borrow().clone())
        .or_else(|| resolve(scope).ok())
    {
        let sink = core.error_sink.borrow().clone();
        if let Some(sink) = sink {
            sink(error);
            return;
        }
    }
    eprintln!("deka reactive: {error}");
}

/// A Copy handle to state. Reads in a reaction subscribe that reaction; writes
/// schedule only subscribers. Handles stay on their owning UI thread.
pub struct Signal<T: 'static> {
    scope: u64,
    slot: usize,
    generation: u64,
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
    let value: Rc<dyn Any> = Rc::new(Value(RefCell::new(value)));
    let slot = if let Some(slot) = core.free_values.borrow_mut().pop() {
        values[slot] = Some(value);
        core.value_generations.borrow_mut()[slot] += 1;
        slot
    } else {
        let slot = values.len();
        values.push(Some(value));
        core.subscribers.borrow_mut().push(BTreeSet::new());
        core.revisions.borrow_mut().push(0);
        core.value_generations.borrow_mut().push(0);
        slot
    };
    if let Some(owner) = core.owners.borrow_mut().last_mut() {
        owner.values.push(slot);
    }
    let generation = core.value_generations.borrow()[slot];
    Signal {
        scope: core.id,
        slot,
        generation,
        marker: PhantomData,
        thread: PhantomData,
    }
}
/// Allocate a signal in the current scope.
pub fn signal<T: 'static>(value: T) -> Signal<T> {
    allocate(Some(value))
}
impl<T: 'static> Signal<T> {
    fn value(self, core: &Core) -> Result<Rc<Value<T>>, ReactiveError> {
        if core.value_generations.borrow().get(self.slot) != Some(&self.generation) {
            return Err(ReactiveError::DisposedSignal);
        }
        core.values
            .borrow()
            .get(self.slot)
            .and_then(Option::as_ref)
            .cloned()
            .and_then(|value| value.downcast::<Value<T>>().ok())
            .ok_or(ReactiveError::DisposedSignal)
    }
    /// Read a live value, or return an error after disposal/teardown. Reads in
    /// reactions subscribe only after the handle has been validated.
    pub fn try_get(self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        let core = resolve(self.scope)?;
        let value = self.value(&core)?;
        let cross_scope = CURRENT.with(|active| {
            active
                .borrow()
                .as_ref()
                .is_some_and(|active| active.id != core.id && !active.reads.borrow().is_empty())
        });
        if cross_scope {
            return Err(ReactiveError::CrossScopeRead);
        }
        let next = value
            .0
            .try_borrow()
            .map_err(|_| ReactiveError::BorrowedValue)?
            .as_ref()
            .ok_or(ReactiveError::Uninitialized)?
            .clone();
        if let Some(reads) = core.reads.borrow_mut().last_mut() {
            reads
                .entry(self.slot)
                .or_insert(core.revisions.borrow()[self.slot]);
        }
        Ok(next)
    }
    /// Checked convenience read also reports a failure to the host sink.
    /// Use try_get to handle a failure without reporting it automatically.
    pub fn get(self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        self.try_get()
            .inspect_err(|error| report(self.scope, *error))
    }
    /// Checked writes notify even for equal values. Retained property bindings
    /// compare their authored values before patching.
    pub fn try_set(self, value: T) -> Result<(), ReactiveError> {
        let core = resolve(self.scope)?;
        let target = self.value(&core)?;
        *target
            .0
            .try_borrow_mut()
            .map_err(|_| ReactiveError::BorrowedValue)? = Some(value);
        core.changed(self.slot);
        Ok(())
    }
    /// Convenience write: reports a stale-handle error rather than unwinding.
    /// Use try_set when the caller needs to handle the error itself.
    pub fn set(self, value: T) {
        if let Err(error) = self.try_set(value) {
            report(self.scope, error);
        }
    }
    pub fn update<R>(self, update: impl FnOnce(&mut T) -> R) -> Result<R, ReactiveError> {
        let core = resolve(self.scope)?;
        let value = self.value(&core)?;
        let _write;
        let mut value = value
            .0
            .try_borrow_mut()
            .map_err(|_| ReactiveError::BorrowedValue)?;
        let value = value.as_mut().ok_or(ReactiveError::Uninitialized)?;
        _write = Write {
            core,
            slot: self.slot,
        };
        Ok(update(value))
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
        if let Err(error) = self.update(|value| *value = !*value) {
            report(self.scope, error);
        }
    }
}
impl<T: AddAssign<T> + 'static> AddAssign<T> for Signal<T> {
    fn add_assign(&mut self, rhs: T) {
        if let Err(error) = self.update(|value| *value += rhs) {
            report(self.scope, error);
        }
    }
}
impl<T: SubAssign<T> + 'static> SubAssign<T> for Signal<T> {
    fn sub_assign(&mut self, rhs: T) {
        if let Err(error) = self.update(|value| *value -= rhs) {
            report(self.scope, error);
        }
    }
}
impl<T: Clone + fmt::Display> fmt::Display for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.get() {
            Ok(value) => value.fmt(f),
            Err(_) => Ok(()),
        }
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
    pub fn try_get(self) -> Result<T, ReactiveError> {
        self.0.try_get()
    }
    pub fn get(self) -> Result<T, ReactiveError> {
        self.0.get()
    }
}
impl<T: Clone + fmt::Display> fmt::Display for Derived<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.get() {
            Ok(value) => value.fmt(f),
            Err(_) => Ok(()),
        }
    }
}
/// Compute immediately, then recompute when its dynamically observed inputs
/// change. Derived reactions settle before effects, avoiding diamond glitches.
pub fn derived<T: PartialEq + 'static>(mut compute: impl FnMut() -> T + 'static) -> Derived<T> {
    let output = allocate(None);
    observe(Kind::Derived, move || {
        let next = compute();
        let Ok(core) = resolve(output.scope) else {
            return;
        };
        let Ok(value) = output.value(&core) else {
            return;
        };
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
    generation: u64,
    marker: PhantomData<Rc<()>>,
}
impl Effect {
    pub fn dispose(self) -> Result<(), ReactiveError> {
        let core = resolve(self.scope)?;
        if core.observer_generations.borrow()[self.slot] == self.generation {
            core.dispose_observer(self.slot);
        }
        Ok(())
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
type ReactiveSink = Rc<dyn Fn(ReactiveError)>;
struct Observer {
    callback: Callback,
    dependencies: BTreeSet<usize>,
    kind: Kind,
    run_owner: Option<ReactiveOwner>,
}
#[derive(Default)]
struct Core {
    id: u64,
    error_sink: RefCell<Option<ReactiveSink>>,
    values: RefCell<Vec<Option<Rc<dyn Any>>>>,
    free_values: RefCell<Vec<usize>>,
    value_generations: RefCell<Vec<u64>>,
    free_observers: RefCell<Vec<usize>>,
    observer_generations: RefCell<Vec<u64>>,
    owners: RefCell<Vec<Owned>>,
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
    let slot = if let Some(slot) = core.free_observers.borrow_mut().pop() {
        core.observer_generations.borrow_mut()[slot] += 1;
        slot
    } else {
        let slot = core.observers.borrow().len();
        core.observers.borrow_mut().push(None);
        core.observer_generations.borrow_mut().push(0);
        slot
    };
    let generation = core.observer_generations.borrow()[slot];
    core.observers.borrow_mut()[slot] = Some(Observer {
        callback: Rc::new(RefCell::new(Box::new(run))),
        dependencies: BTreeSet::new(),
        kind,
        run_owner: None,
    });
    if let Some(owner) = core.owners.borrow_mut().last_mut() {
        owner.effects.push(Effect {
            scope: core.id,
            slot,
            generation,
            marker: PhantomData,
        });
    }
    // Hold the batch boundary while installing nested reactions. In particular,
    // a derived value must finish its read capture before its dependents run.
    Scope(core.clone()).batch(|| core.evaluate(slot));
    Effect {
        scope: core.id,
        slot,
        generation,
        marker: PhantomData,
    }
}
impl Core {
    fn dispose_observer(&self, slot: usize) {
        let observer = self.observers.borrow_mut()[slot].take();
        if let Some(observer) = observer {
            for dependency in &observer.dependencies {
                self.subscribers.borrow_mut()[*dependency].remove(&slot);
            }
            self.dirty.borrow_mut().remove(&slot);
            self.free_observers.borrow_mut().push(slot);
        }
    }
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
        let execution = self.observers.borrow_mut()[slot]
            .as_mut()
            .map(|o| (o.callback.clone(), o.run_owner.take()));
        let Some((callback, previous_owner)) = execution else {
            return;
        };
        drop(previous_owner);
        let generation = self.observer_generations.borrow()[slot];
        let (_, owner) = Scope(self.clone()).run(|| {
            owned(|| {
                self.reads.borrow_mut().push(BTreeMap::new());
                let _capture = Capture {
                    core: self.clone(),
                    slot,
                    generation,
                };
                callback.borrow_mut()();
            })
        });
        if self.observer_generations.borrow()[slot] == generation {
            let mut observers = self.observers.borrow_mut();
            if let Some(observer) = observers[slot].as_mut() {
                observer.run_owner = Some(owner);
                return;
            }
        }
        drop(owner);
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
    generation: u64,
}
impl Drop for Capture {
    fn drop(&mut self) {
        let reads = self
            .core
            .reads
            .borrow_mut()
            .pop()
            .expect("reaction read capture");
        if self.core.observer_generations.borrow()[self.slot] != self.generation {
            return;
        }
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

// Dynamic children own allocations created while building their registrations.
// Each reaction execution owns its allocations until rerun or disposal.
#[derive(Default)]
struct Owned {
    values: Vec<usize>,
    effects: Vec<Effect>,
}
pub(crate) struct ReactiveOwner {
    core: Weak<Core>,
    owned: Owned,
}
impl Drop for ReactiveOwner {
    fn drop(&mut self) {
        let Some(core) = self.core.upgrade() else {
            return;
        };
        for effect in self.owned.effects.drain(..) {
            // Idempotent disposal; DroppedScope means the whole scope is
            // already gone, so teardown has no live error to report.
            let _ = effect.dispose();
        }
        for slot in self.owned.values.drain(..) {
            core.values.borrow_mut()[slot] = None;
            core.revisions.borrow_mut()[slot] = 0;
            core.subscribers.borrow_mut()[slot].clear();
            for observer in core.observers.borrow_mut().iter_mut().flatten() {
                observer.dependencies.remove(&slot);
            }
            for reads in core.reads.borrow_mut().iter_mut() {
                reads.remove(&slot);
            }
            core.free_values.borrow_mut().push(slot);
        }
    }
}
pub(crate) fn owned<R>(run: impl FnOnce() -> R) -> (R, ReactiveOwner) {
    let core = current();
    core.owners.borrow_mut().push(Owned::default());
    struct CaptureOwner(Option<Rc<Core>>);
    impl Drop for CaptureOwner {
        fn drop(&mut self) {
            if let Some(core) = self.0.take() {
                let owned = core.owners.borrow_mut().pop().expect("allocation owner");
                drop(ReactiveOwner {
                    core: Rc::downgrade(&core),
                    owned,
                });
            }
        }
    }
    let mut guard = CaptureOwner(Some(core.clone()));
    let value = run();
    let owned = core.owners.borrow_mut().pop().expect("allocation owner");
    guard.0 = None;
    (
        value,
        ReactiveOwner {
            core: Rc::downgrade(&core),
            owned,
        },
    )
}

#[cfg(test)]
mod slot_tests {
    use super::*;
    use crate::{UiApp, View};
    #[test]
    fn outlived_dynamic_signal_reports_without_panicking() {
        let handle = Rc::new(Cell::new(None));
        let captured = handle.clone();
        let app = UiApp::new(move || {
            View::dynamic(move || {
                let value = signal(7);
                captured.set(Some(value));
                View::text("child")
            })
        });
        let stale = handle.get().unwrap();
        drop(app);
        assert_eq!(stale.try_get(), Err(ReactiveError::DroppedScope));
        assert_eq!(stale.try_set(9), Err(ReactiveError::DroppedScope));
        assert_eq!(stale.get(), Err(ReactiveError::DroppedScope));
        stale.set(9);
        let scope = Scope::new();
        let errors = Rc::new(RefCell::new(Vec::new()));
        let output = errors.clone();
        scope.on_error(move |error| output.borrow_mut().push(error));
        let (stale, owner) = scope.run(|| owned(|| signal(7)));
        drop(owner);
        assert_eq!(stale.try_get(), Err(ReactiveError::DisposedSignal));
        assert_eq!(stale.try_set(9), Err(ReactiveError::DisposedSignal));
        assert_eq!(stale.get(), Err(ReactiveError::DisposedSignal));
        stale.set(9);
        assert_eq!(*errors.borrow(), [ReactiveError::DisposedSignal; 2]);
    }
    #[test]
    fn released_slots_remove_outer_dependencies_and_reset_revisions() {
        let scope = Scope::new();
        let (child, owner) = scope.run(|| owned(|| signal(7)));
        let calls = Rc::new(Cell::new(0));
        let output = calls.clone();
        let observer = scope.run(|| {
            effect(move || {
                let _ = child.try_get();
                output.set(output.get() + 1);
            })
        });
        child.set(8);
        drop(owner);
        assert_eq!(scope.0.revisions.borrow()[child.slot], 0);
        assert!(
            !scope.0.observers.borrow()[observer.slot]
                .as_ref()
                .unwrap()
                .dependencies
                .contains(&child.slot)
        );
        let next = scope.run(|| signal(10));
        assert_eq!(next.slot, child.slot);
        next.set(11);
        assert_eq!(calls.get(), 2);
    }
    #[test]
    fn effect_rerun_allocations_are_bounded_and_released_with_owner() {
        let scope = Scope::new();
        let trigger = scope.run(|| signal(0));
        let last = Rc::new(Cell::new(None));
        let output = last.clone();
        let (_, owner) = scope.run(|| {
            owned(|| {
                effect(move || {
                    trigger.get().unwrap();
                    output.set(Some(signal(42)));
                })
            })
        });
        for n in 1..100 {
            let previous = last.get().unwrap();
            trigger.set(n);
            assert_eq!(previous.try_get(), Err(ReactiveError::DisposedSignal));
            assert!(scope.0.values.borrow().len() <= 2);
        }
        drop(owner);
        assert_eq!(
            last.get().unwrap().try_get(),
            Err(ReactiveError::DisposedSignal)
        );
    }
    #[test]
    fn disposed_signal_returns_error_even_after_its_slot_is_reused() {
        let scope = Scope::new();
        let (old, owner) = scope.run(|| owned(|| signal(1)));
        drop(owner);
        assert_eq!(old.get(), Err(ReactiveError::DisposedSignal));
        let new = scope.run(|| signal(2));
        assert_eq!(old.slot, new.slot);
        assert_eq!(old.get(), Err(ReactiveError::DisposedSignal));
        assert_eq!(old.try_set(3), Err(ReactiveError::DisposedSignal));
        assert_eq!(new.get(), Ok(2));
    }
    #[test]
    fn dynamic_list_reuses_all_reactive_slots_and_releases_values() {
        let input = Rc::new(Cell::new(None));
        let output = input.clone();
        let weak = Rc::new(RefCell::new(Vec::new()));
        let weak_output = weak.clone();
        let app = UiApp::new(move || {
            let length = signal(8);
            output.set(Some(length));
            View::element("view").child(View::dynamic(move || {
                let length = length.get().unwrap();
                (0..length)
                    .map(|_| {
                        let value = Rc::new(());
                        weak_output.borrow_mut().push(Rc::downgrade(&value));
                        let state = signal(value);
                        let count = signal(1);
                        let doubled = derived(move || count.get().unwrap() * 2);
                        effect(move || {
                            state.get().unwrap();
                        });
                        View::live_text(move || doubled.get().unwrap().to_string())
                    })
                    .collect::<Vec<_>>()
            }))
        });
        let length = input.get().unwrap();
        let core = resolve(length.scope).unwrap();
        for index in 0..100 {
            length.set(if index % 2 == 0 { 4 } else { 8 });
            assert!(core.values.borrow().len() <= 49);
            assert!(core.subscribers.borrow().len() <= 49);
            assert!(core.observers.borrow().len() <= 65);
            assert!(
                weak.borrow()
                    .iter()
                    .filter(|value| value.strong_count() > 0)
                    .count()
                    <= 8
            );
            assert_eq!(app.tree().children.len(), length.get().unwrap());
        }
        drop(app);
        drop(core);
        assert!(weak.borrow().iter().all(|value| value.upgrade().is_none()));
    }
    #[test]
    fn disposed_effect_handle_cannot_dispose_reused_registration() {
        let scope = Scope::new();
        let calls = Rc::new(Cell::new(0));
        let input = scope.run(|| signal(0));
        let old = scope.run(|| {
            effect(move || {
                input.get().unwrap();
            })
        });
        old.dispose().unwrap();
        let output = calls.clone();
        let next = scope.run(|| {
            effect(move || {
                input.get().unwrap();
                output.set(output.get() + 1);
            })
        });
        assert_eq!(old.slot, next.slot);
        old.dispose().unwrap();
        input.set(1);
        assert_eq!(calls.get(), 2);
        next.dispose().unwrap();
        assert_eq!(scope.0.free_observers.borrow().len(), 1);
    }
}
