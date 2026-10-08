//! Transient Rust builders and reactive bindings on the shared native store.
use crate::{ComponentState, Derived, Effect, NodeRef, Scope, Signal, ViewTree, effect};
use deka_native_ir::{
    Node, WireNode,
    tree::{NodeHandle, Tree},
};
use deka_native_ui::Application;
use std::any::Any;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fmt::Display,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventKind {
    Click,
    Input,
    KeyDown,
    ContextMenu,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Click,
    Input(String),
    KeyDown(String),
    ContextMenu { x: f32, y: f32 },
}
impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Self::Click => EventKind::Click,
            Self::Input(_) => EventKind::Input,
            Self::KeyDown(_) => EventKind::KeyDown,
            Self::ContextMenu { .. } => EventKind::ContextMenu,
        }
    }
}

#[doc(hidden)]
pub struct Static;
#[doc(hidden)]
pub struct Live;
#[doc(hidden)]
pub struct Sequence;
#[doc(hidden)]
pub struct Computed<M>(std::marker::PhantomData<M>);
#[doc(hidden)]
pub struct Tracked;
#[doc(hidden)]
pub struct TwoWay;
#[doc(hidden)]
pub trait IntoValue<M> {
    fn apply(self, view: View) -> View;
}
impl IntoValue<Static> for String {
    fn apply(self, view: View) -> View {
        view.attr("value", self)
    }
}
impl IntoValue<Static> for &str {
    fn apply(self, view: View) -> View {
        view.attr("value", self)
    }
}
impl<F, T> IntoValue<Live> for F
where
    F: FnMut() -> T + 'static,
    T: Display,
{
    fn apply(self, view: View) -> View {
        view.attr("value", self)
    }
}
impl IntoValue<TwoWay> for Signal<String> {
    fn apply(self, mut view: View) -> View {
        if let Some(element) = view.element_mut() {
            element.value_signal = Some(self);
        }
        view.attr("value", move || self.get().unwrap_or_default())
    }
}
#[doc(hidden)]
pub enum Attribute {
    Value(String),
    Binding(Box<dyn FnMut() -> String>),
}
#[doc(hidden)]
pub trait IntoAttribute<M> {
    fn into_attribute(self) -> Attribute;
}
impl<T: Display> IntoAttribute<Static> for T {
    fn into_attribute(self) -> Attribute {
        Attribute::Value(self.to_string())
    }
}
impl<F, T> IntoAttribute<Live> for F
where
    F: FnMut() -> T + 'static,
    T: Display,
{
    fn into_attribute(mut self) -> Attribute {
        Attribute::Binding(Box::new(move || self().to_string()))
    }
}

/// An authoring value consumed when mounted. It is not a second retained tree.
pub struct View(Builder);
enum Builder {
    Element(Element),
    Text(Attribute),
    Fragment(Vec<View>),
    Dynamic(Box<dyn FnMut() -> View>),
    Error(String),
    Ref(Box<View>, NodeRef),
    State(Box<View>, Rc<dyn Any>),
}
struct Element {
    value_signal: Option<Signal<String>>,
    tag: String,
    attributes: BTreeMap<String, Attribute>,
    toggles: BTreeMap<String, Box<dyn FnMut() -> bool>>,
    events: BTreeMap<EventKind, Box<dyn FnMut(Event)>>,
    children: Vec<View>,
}
impl View {
    /// Capture a Display value for a live quoted-text interpolation.
    /// Cloning also supports owned values reused in more than one text binding.
    pub fn interpolate<T: Clone + std::fmt::Display + 'static>(value: &T) -> Self {
        let value = value.clone();
        Self::live_text(move || format!("{value}"))
    }
    pub fn node_ref(self, reference: NodeRef) -> Self {
        Self(Builder::Ref(Box::new(self), reference))
    }
    pub fn with_state<T: 'static>(self, state: ComponentState<T>) -> Self {
        Self(Builder::State(Box::new(self), state.erase()))
    }
    fn single_root(&self) -> bool {
        match &self.0 {
            Builder::Element(_) | Builder::Text(_) => true,
            Builder::Ref(view, _) | Builder::State(view, _) => view.single_root(),
            _ => false,
        }
    }
    pub fn from_child<V, M>(value: V) -> Self
    where
        V: IntoView<M>,
    {
        value.into_view()
    }
    pub fn element(tag: impl Into<String>) -> Self {
        Self(Builder::Element(Element {
            tag: tag.into(),
            value_signal: None,
            attributes: BTreeMap::new(),
            toggles: BTreeMap::new(),
            events: BTreeMap::new(),
            children: vec![],
        }))
    }
    pub fn text(value: impl Display) -> Self {
        Self(Builder::Text(Attribute::Value(value.to_string())))
    }
    pub fn live_text<F, T>(value: F) -> Self
    where
        F: FnMut() -> T + 'static,
        T: Display,
    {
        Self(Builder::Text(value.into_attribute()))
    }
    pub fn fragment(children: impl IntoIterator<Item = View>) -> Self {
        Self(Builder::Fragment(children.into_iter().collect()))
    }
    pub fn dynamic<F, V, M>(mut value: F) -> Self
    where
        F: FnMut() -> V + 'static,
        V: IntoView<M>,
    {
        Self(Builder::Dynamic(Box::new(move || value().into_view())))
    }
    /// A string signal binds both directions; strings and closures bind one direction.
    pub fn value<V: IntoValue<M>, M>(self, value: V) -> Self {
        value.apply(self)
    }
    pub fn attr<V, M>(mut self, name: &str, value: V) -> Self
    where
        V: IntoAttribute<M>,
    {
        if !deka_native_ir::is_supported_attribute(name) {
            return Self(Builder::Error(format!(
                "unsupported native attribute {name}"
            )));
        }
        if let Some(element) = self.element_mut() {
            element
                .attributes
                .insert(name.into(), value.into_attribute());
        }
        self
    }
    pub fn class<V, M>(mut self, name: &str, value: V) -> Self
    where
        V: IntoToggle<M>,
    {
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Self(Builder::Error("class toggle requires one utility".into()));
        }
        if let Err(error) = deka_native_ir::apply_classes(&mut Default::default(), name) {
            return Self(Builder::Error(error));
        }
        if let Some(element) = self.element_mut() {
            element.toggles.insert(name.into(), value.into_toggle());
        }
        self
    }
    pub fn on(mut self, kind: EventKind, event: impl FnMut(Event) + 'static) -> Self {
        if let Some(element) = self.element_mut() {
            element.events.insert(kind, Box::new(event));
        }
        self
    }
    pub fn on_click(self, event: impl FnMut(Event) + 'static) -> Self {
        self.on(EventKind::Click, event)
    }
    pub fn child<V, M>(mut self, child: V) -> Self
    where
        V: IntoView<M>,
    {
        if let Some(element) = self.element_mut() {
            element.children.push(child.into_view());
        }
        self
    }
    pub fn children(mut self, children: impl IntoIterator<Item = View>) -> Self {
        if let Some(element) = self.element_mut() {
            element.children.push(Self::fragment(children));
        }
        self
    }
    fn element_mut(&mut self) -> Option<&mut Element> {
        if !matches!(
            self.0,
            Builder::Element(_) | Builder::Ref(_, _) | Builder::State(_, _)
        ) {
            if !matches!(self.0, Builder::Error(_)) {
                self.0 = Builder::Error("attributes/events/children require an element".into());
            }
            return None;
        }
        match &mut self.0 {
            Builder::Element(element) => Some(element),
            Builder::Ref(view, _) | Builder::State(view, _) => view.element_mut(),
            _ => None,
        }
    }
}

/// Component children are built once, in the receiving component's scope.
pub type Children = Box<dyn FnOnce() -> View>;
#[doc(hidden)]
pub trait IntoView<M> {
    fn into_view(self) -> View;
}
impl IntoView<Static> for View {
    fn into_view(self) -> View {
        self
    }
}
impl<T: IntoIterator<Item = View>> IntoView<Sequence> for T {
    fn into_view(self) -> View {
        View::fragment(self)
    }
}
impl<F, V, M> IntoView<Computed<M>> for F
where
    F: FnMut() -> V + 'static,
    V: IntoView<M>,
{
    fn into_view(self) -> View {
        View::dynamic(self)
    }
}
impl<T: Clone + Display + 'static> IntoView<Tracked> for Signal<T> {
    fn into_view(self) -> View {
        View::live_text(move || format!("{self}"))
    }
}
impl<T: Clone + Display + 'static> IntoView<Tracked> for Derived<T> {
    fn into_view(self) -> View {
        View::live_text(move || format!("{self}"))
    }
}
macro_rules! scalar_children {
    ($($ty:ty),* $(,)?) => {$(impl IntoView<Static> for $ty { fn into_view(self) -> View { View::text(self) } })*};
}
scalar_children!(
    String, &str, bool, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32,
    f64
);
#[doc(hidden)]
pub trait IntoToggle<M> {
    fn into_toggle(self) -> Box<dyn FnMut() -> bool>;
}
impl IntoToggle<Static> for bool {
    fn into_toggle(self) -> Box<dyn FnMut() -> bool> {
        Box::new(move || self)
    }
}
impl IntoToggle<Tracked> for Signal<bool> {
    fn into_toggle(self) -> Box<dyn FnMut() -> bool> {
        Box::new(move || self.get().unwrap_or(false))
    }
}
impl IntoToggle<Tracked> for Derived<bool> {
    fn into_toggle(self) -> Box<dyn FnMut() -> bool> {
        Box::new(move || self.get().unwrap_or(false))
    }
}
impl<F: FnMut() -> bool + 'static> IntoToggle<Live> for F {
    fn into_toggle(self) -> Box<dyn FnMut() -> bool> {
        Box::new(self)
    }
}

#[doc(hidden)]
pub struct NoProps;
#[doc(hidden)]
pub struct Props<P>(std::marker::PhantomData<P>);
#[doc(hidden)]
pub trait BuildApp<M> {
    fn build(self) -> View;
}
impl<F: FnOnce() -> View> BuildApp<NoProps> for F {
    fn build(self) -> View {
        self()
    }
}
impl<F: FnOnce(P) -> View, P: Default> BuildApp<Props<P>> for F {
    fn build(self) -> View {
        self(P::default())
    }
}

/// An operational error with its authored structural path and binding name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiError {
    pub path: Vec<usize>,
    pub binding: String,
    pub message: String,
}
impl std::fmt::Display for UiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "node {:?} binding {}: {}",
            self.path, self.binding, self.message
        )
    }
}
/// Called once per error occurrence, on the owning UI thread.
pub type ErrorSink = Rc<dyn Fn(&UiError)>;
const ERROR_BUFFER_LIMIT: usize = 128;

/// Host launch configuration. The default sink logs errors to stderr.
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
pub struct LaunchOptions {
    pub window: deka_native_ui::window::Options,
    pub reduced_motion: bool,
    pub error_sink: Option<ErrorSink>,
}

#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
impl LaunchOptions {
    pub fn new(window: deka_native_ui::window::Options) -> Self {
        Self {
            window,
            reduced_motion: false,
            error_sink: None,
        }
    }
}

pub(crate) type PublishedState = (Anchor, Rc<dyn Any>);
pub(crate) type Anchor = Rc<RefCell<Option<NodeHandle>>>;

type Callback = Rc<RefCell<Box<dyn FnMut(Event)>>>;
type TextIndex = (usize, Vec<(NodeHandle, bool)>);
struct Listener {
    controlled_value: bool,
    anchor: Anchor,
    kind: EventKind,
    callback: Callback,
}
#[derive(Default)]
pub(crate) struct Context {
    pub(crate) tree: RefCell<Tree>,
    pub(crate) scope_id: u64,
    pub(crate) session_id: u64,
    pub(crate) states: RefCell<BTreeMap<usize, PublishedState>>,
    next_state: Cell<usize>,
    listeners: RefCell<BTreeMap<usize, Listener>>,
    next_listener: Cell<usize>,
    text_index: RefCell<Option<TextIndex>>,
    patches: Cell<usize>,
    pending_frame: Cell<bool>,
    waker: RefCell<Option<deka_native_ui::Waker>>,
    errors: RefCell<Vec<String>>,
    error_sink: RefCell<Option<ErrorSink>>,
    error_location: RefCell<(Vec<usize>, String)>,
    event_owners: RefCell<Vec<crate::reactive::ReactiveOwner>>,
}
impl Context {
    pub(crate) fn snapshot(&self) -> Node {
        self.tree.borrow().snapshot().unwrap_or_else(|| Node {
            id: "view/empty".into(),
            style: Default::default(),
            text: None,
            on_click: None,
            children: vec![],
        })
    }
    pub(crate) fn is_attached(&self, node: &NodeHandle) -> bool {
        let root = self.tree.borrow().root.clone();
        let mut current = Some(node.clone());
        while let Some(node) = current {
            if root.as_ref().is_some_and(|root| root == &node) {
                return true;
            }
            current = node.parent();
        }
        false
    }
    pub(crate) fn node_changed(&self, node: &NodeHandle, changed: bool) {
        if changed && self.is_attached(node) {
            self.changed(true);
        }
    }
    fn publish(&self, anchor: Anchor, state: Rc<dyn Any>) -> usize {
        let token = self.next_state.get();
        self.next_state.set(token + 1);
        self.states.borrow_mut().insert(token, (anchor, state));
        token
    }
    fn report(&self, path: &[usize], binding: &str, message: String) {
        let error = UiError {
            path: path.to_vec(),
            binding: binding.into(),
            message,
        };
        {
            let mut errors = self.errors.borrow_mut();
            if errors.len() == ERROR_BUFFER_LIMIT {
                errors.remove(0);
            }
            errors.push(error.message.clone());
        }
        let sink = self.error_sink.borrow().clone();
        if let Some(sink) = sink {
            sink(&error);
        } else {
            eprintln!("deka ui: {error}");
        }
    }
    pub(crate) fn report_current(&self, message: String) {
        let (path, binding) = self.error_location.borrow().clone();
        self.report(&path, &binding, message);
    }
    fn at<R>(&self, path: &[usize], binding: &str, run: impl FnOnce() -> R) -> R {
        struct Location<'a>(&'a Context, Option<(Vec<usize>, String)>);
        impl Drop for Location<'_> {
            fn drop(&mut self) {
                self.0.error_location.replace(self.1.take().unwrap());
            }
        }
        let previous = self.error_location.replace((path.to_vec(), binding.into()));
        let _location = Location(self, Some(previous));
        crate::retained::with_session(self.session_id, run)
    }
    #[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
    pub(crate) fn native_event<R>(&self, scope: &Scope, run: impl FnOnce() -> R) -> R {
        self.owned_event(scope, &[], "native handler", run)
    }
    fn owned_event<R>(
        &self,
        scope: &Scope,
        path: &[usize],
        binding: &str,
        run: impl FnOnce() -> R,
    ) -> R {
        let (result, owner) = self.at(path, binding, || {
            scope.batch(|| crate::reactive::owned(run))
        });
        if !owner.is_empty() {
            self.event_owners.borrow_mut().push(owner);
        }
        result
    }
    fn changed(&self, changed: bool) {
        if changed {
            self.patches.set(self.patches.get() + 1);
            if !self.pending_frame.replace(true)
                && let Some(waker) = self.waker.borrow().as_ref()
            {
                waker.wake();
            }
        }
    }
    fn listen(
        &self,
        anchor: Anchor,
        kind: EventKind,
        event: Box<dyn FnMut(Event)>,
        controlled_value: bool,
    ) -> usize {
        self.text_index.borrow_mut().take();
        let token = self.next_listener.get();
        self.next_listener
            .set(token.checked_add(1).expect("event identity exhausted"));
        self.listeners.borrow_mut().insert(
            token,
            Listener {
                controlled_value,
                anchor,
                kind,
                callback: Rc::new(RefCell::new(event)),
            },
        );
        token
    }
}
/// Registration ownership only: no node topology or renderer snapshots.
#[derive(Default)]
struct Registrations {
    effects: Vec<Effect>,
    states: Vec<usize>,
    reactive: Vec<crate::reactive::ReactiveOwner>,
    events: Vec<usize>,
    groups: Vec<Rc<RefCell<Registrations>>>,
    context: Weak<Context>,
}
impl Registrations {
    fn extend(&mut self, mut other: Self) {
        self.effects.append(&mut other.effects);
        self.states.append(&mut other.states);
        self.reactive.append(&mut other.reactive);
        self.events.append(&mut other.events);
        self.groups.append(&mut other.groups);
    }
}
impl Drop for Registrations {
    fn drop(&mut self) {
        for effect in self.effects.drain(..) {
            // Teardown is idempotent; DroppedScope means all registrations are
            // already gone, so no live operation or error needs reporting.
            let _ = effect.dispose();
        }
        if let Some(context) = self.context.upgrade() {
            for token in self.states.drain(..) {
                context.states.borrow_mut().remove(&token);
            }
            for token in self.events.drain(..) {
                context.listeners.borrow_mut().remove(&token);
                context.text_index.borrow_mut().take();
            }
        }
    }
}
struct Prepared {
    wires: Vec<WireNode>,
    slots: Vec<Vec<usize>>,
    anchors: Vec<(Vec<usize>, Anchor)>,
    registrations: Registrations,
    root_anchors: Vec<Anchor>,
    references: Vec<(NodeRef, Anchor)>,
}
impl Prepared {
    fn new(context: &Rc<Context>) -> Self {
        Self {
            root_anchors: vec![],
            references: vec![],
            wires: vec![],
            slots: vec![],
            anchors: vec![],
            registrations: Registrations {
                context: Rc::downgrade(context),
                effects: vec![],
                states: vec![],
                reactive: vec![],
                events: vec![],
                groups: vec![],
            },
        }
    }
    fn extend(&mut self, mut child: Self) {
        self.root_anchors.append(&mut child.root_anchors);
        self.references.append(&mut child.references);
        self.wires.append(&mut child.wires);
        self.slots.append(&mut child.slots);
        self.anchors.append(&mut child.anchors);
        self.registrations.extend(child.registrations);
    }
    fn attach(&self, roots: &[NodeHandle], context: &Rc<Context>) {
        fn visit(node: &NodeHandle, nodes: &mut BTreeMap<Vec<usize>, NodeHandle>) {
            nodes.insert(node.slot(), node.clone());
            for child in node.0.borrow().children.clone() {
                visit(&child, nodes);
            }
        }
        let mut nodes = BTreeMap::new();
        for root in roots {
            visit(root, &mut nodes);
        }
        for (slot, anchor) in &self.anchors {
            *anchor.borrow_mut() = Some(
                nodes
                    .get(slot)
                    .expect("missing mounted structural slot")
                    .clone(),
            );
        }
        for (reference, anchor) in &self.references {
            if let Some(node) = anchor.borrow().clone() {
                reference.capture(node, context);
            }
        }
    }
}

fn bind(
    value: Attribute,
    path: &[usize],
    binding: &str,
    anchor: &Anchor,
    context: &Rc<Context>,
    registrations: &mut Registrations,
    patch: impl Fn(&NodeHandle, String) -> Result<bool, String> + 'static,
) -> String {
    match value {
        Attribute::Value(value) => value,
        Attribute::Binding(mut getter) => {
            let initial = Rc::new(RefCell::new(String::new()));
            let output = initial.clone();
            let target = anchor.clone();
            let context = Rc::downgrade(context);
            let path = path.to_vec();
            let binding = binding.to_owned();
            registrations.effects.push(effect(move || {
                let Some(context) = context.upgrade() else {
                    return;
                };
                let value = context.at(&path, &binding, &mut getter);
                if let Some(node) = target.borrow().as_ref() {
                    match patch(node, value) {
                        Ok(changed) => context.changed(changed),
                        Err(error) => context.report(&path, &binding, error),
                    }
                } else {
                    *output.borrow_mut() = value;
                }
            }));
            initial.take()
        }
    }
}
fn prepare(
    view: View,
    path: Vec<usize>,
    parent: Anchor,
    context: &Rc<Context>,
) -> Result<Prepared, String> {
    let mut prepared = Prepared::new(context);
    match view.0 {
        Builder::Error(error) => return Err(format!("node {path:?} builder: {error}")),
        Builder::Ref(view, reference) => {
            reference.check(context.session_id)?;
            let mut next = prepare(*view, path, parent, context)?;
            if next.root_anchors.len() != 1 {
                return Err("node_ref requires one retained root".into());
            }
            next.references
                .push((reference, next.root_anchors[0].clone()));
            return Ok(next);
        }
        Builder::State(view, state) => {
            let mut next = prepare(*view, path, parent, context)?;
            for anchor in &next.root_anchors {
                next.registrations
                    .states
                    .push(context.publish(anchor.clone(), state.clone()));
            }
            return Ok(next);
        }
        Builder::Fragment(children) => {
            for (index, child) in children.into_iter().enumerate() {
                let mut slot = path.clone();
                slot.push(index);
                prepared.extend(prepare(child, slot, parent.clone(), context)?);
            }
        }
        Builder::Dynamic(mut getter) => {
            let initial = Rc::new(RefCell::new(None));
            let output = initial.clone();
            let group = Rc::new(RefCell::new(Registrations::default()));
            let owner = group.clone();
            let prefix = path;
            let weak = Rc::downgrade(context);
            prepared.registrations.effects.push(effect(move || {
                let Some(context) = weak.upgrade() else {
                    return;
                };
                let mut slot = prefix.clone();
                slot.push(0);
                let (next, reactive) = crate::reactive::owned(|| {
                    context.at(&prefix, "children", || {
                        prepare(getter(), slot, parent.clone(), &context)
                    })
                });
                let mut next = match next {
                    Ok(next) => next,
                    Err(error) => {
                        context.report(&prefix, "children", error);
                        return;
                    }
                };
                next.registrations.reactive.push(reactive);
                let target = parent.borrow().clone();
                if let Some(parent) = target {
                    let wires = std::mem::take(&mut next.wires);
                    let replacement = context.tree.borrow_mut().replace_slot(
                        &parent,
                        &prefix,
                        wires,
                        &next.slots,
                    );
                    let roots = match replacement {
                        Ok(roots) => roots,
                        Err(error) => {
                            context.report(&prefix, "children", error);
                            return;
                        }
                    };
                    next.attach(&roots, &context);
                    *owner.borrow_mut() = next.registrations;
                    context.changed(true);
                } else {
                    *output.borrow_mut() = Some(next);
                }
            }));
            let Some(mut next) = initial.borrow_mut().take() else {
                // The first evaluation failed; preserve an empty structural slot.
                prepared.registrations.groups.push(group);
                return Ok(prepared);
            };
            *group.borrow_mut() = next.registrations;
            next.registrations = Registrations::default();
            prepared.registrations.groups.push(group);
            prepared.extend(next);
        }
        Builder::Text(value) => {
            let anchor = Anchor::default();
            prepared.root_anchors.push(anchor.clone());
            let text = bind(
                value,
                &path,
                "text",
                &anchor,
                context,
                &mut prepared.registrations,
                |node, value| Ok(node.patch_text(value)),
            );
            prepared.slots.push(path.clone());
            prepared.anchors.push((path, anchor));
            prepared.wires.push(WireNode {
                text: Some(text),
                ..Default::default()
            });
        }
        Builder::Element(mut element) => {
            let anchor = Anchor::default();
            let mut wire = WireNode {
                tag: element.tag,
                ..Default::default()
            };
            let base = element.attributes.remove("className");
            let has_classes = base.is_some() || !element.toggles.is_empty();
            let base = base.unwrap_or_else(|| Attribute::Value(String::new()));
            let classes = if element.toggles.is_empty() {
                base
            } else {
                let mut toggles = element.toggles;
                let mut base = base;
                Attribute::Binding(Box::new(move || {
                    let value = match &mut base {
                        Attribute::Value(value) => value.clone(),
                        Attribute::Binding(getter) => getter(),
                    };
                    let mut tokens: Vec<_> = value.split_whitespace().map(str::to_owned).collect();
                    for (class, getter) in &mut toggles {
                        tokens.retain(|token| token != class);
                        if getter() {
                            tokens.push(class.clone());
                        }
                    }
                    tokens.join(" ")
                }))
            };
            wire.classes = bind(
                classes,
                &path,
                "className",
                &anchor,
                context,
                &mut prepared.registrations,
                NodeHandle::patch_classes,
            );
            if has_classes {
                wire.attributes
                    .insert("className".into(), wire.classes.clone());
            }
            let controlled_value =
                matches!(element.attributes.get("value"), Some(Attribute::Binding(_)));
            for (name, value) in element.attributes {
                let property = name.clone();
                let value = bind(
                    value,
                    &path,
                    &name,
                    &anchor,
                    context,
                    &mut prepared.registrations,
                    move |node, value| node.patch_attribute(&property, value),
                );
                wire.attributes.insert(name, value);
            }
            if let Some(value) = element.value_signal {
                if !matches!(wire.tag.as_str(), "input" | "textarea") {
                    return Err("two-way value requires input or textarea".into());
                }
                let mut user = element.events.remove(&EventKind::Input);
                let target = anchor.clone();
                let owner = Rc::downgrade(context);
                element.events.insert(
                    EventKind::Input,
                    Box::new(move |event| {
                        if let Event::Input(text) = &event {
                            value.set(text.clone());
                        }
                        if let Some(user) = &mut user {
                            user(event);
                        }
                        // A handler may normalize or reject the edit in this batch,
                        // including restoring the previous authored value.
                        let node = target.borrow().clone();
                        if let (Some(node), Some(context), Ok(text)) =
                            (node, owner.upgrade(), value.try_get())
                        {
                            match node.set_attribute("value", text) {
                                Ok(changed) => context.node_changed(&node, changed),
                                Err(error) => context.report(&node.slot(), "value", error),
                            }
                        }
                    }),
                );
            }
            for (kind, event) in element.events {
                let token = context.listen(anchor.clone(), kind, event, controlled_value);
                prepared.registrations.events.push(token);
                if kind == EventKind::Click {
                    wire.handler = Some(token);
                }
            }
            wire.style()
                .map_err(|error| format!("node {path:?} className: {error}"))?;
            prepared.slots.push(path.clone());
            prepared.anchors.push((path.clone(), anchor.clone()));
            for (index, child) in element.children.into_iter().enumerate() {
                let mut slot = path.clone();
                slot.push(index);
                let mut child = prepare(child, slot, anchor.clone(), context)?;
                wire.children.append(&mut child.wires);
                prepared.extend(child);
            }
            prepared.root_anchors = vec![anchor];
            prepared.wires.push(wire);
        }
    }
    Ok(prepared)
}
/// Owns an app's reactive scope, binding registrations, event closures and shared
/// retained store. Rendering takes a snapshot and performs no reactive work.
pub struct UiApp {
    registrations: Option<Registrations>,
    scope: Scope,
    context: Rc<Context>,
    routes: RefCell<Vec<usize>>,
    patch_passes: Cell<usize>,
}
impl UiApp {
    pub fn new<A, M>(app: A) -> Self
    where
        A: BuildApp<M>,
    {
        Self::new_with_error_sink(app, None)
    }
    /// Install the sink before mounting, including initial binding errors.
    pub fn new_with_error_sink<A, M>(app: A, sink: Option<ErrorSink>) -> Self
    where
        A: BuildApp<M>,
    {
        Self::in_scope(Scope::new(), app, sink)
    }
    /// Mount a separate retained tree in an existing app scope. Signals are
    /// shared; local allocations and registrations are owned by this mount.
    pub fn new_in_scope<A, M>(scope: &Scope, app: A) -> Self
    where
        A: BuildApp<M>,
    {
        Self::in_scope(scope.clone(), app, None)
    }
    pub(crate) fn in_scope<A, M>(scope: Scope, app: A, sink: Option<ErrorSink>) -> Self
    where
        A: BuildApp<M>,
    {
        let context = Rc::new(Context {
            scope_id: scope.id(),
            session_id: crate::retained::next_session_id(),
            ..Context::default()
        });
        crate::retained::register(&context);
        context.error_sink.replace(sink);
        context.error_location.replace((vec![], "reactive".into()));
        scope.ensure_error_sink(|error| {
            if let Some(context) =
                crate::retained::active_context().and_then(|context| context.upgrade())
            {
                context.report_current(error.to_string());
            } else {
                eprintln!("deka reactive: {error}");
            }
        });
        let (mut registrations, owner) = scope.run(|| {
            context.at(&[], "mount", || {
                crate::reactive::owned(|| {
                    let view = app.build();
                    let view = if view.single_root() {
                        view
                    } else {
                        View::element("view").child(view)
                    };
                    let mut prepared = match prepare(view, vec![], Anchor::default(), &context) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            context.report(&[], "mount", error);
                            Prepared::new(&context)
                        }
                    };
                    if prepared.wires.is_empty() {
                        prepared.slots.push(vec![]);
                    }
                    let wire = prepared.wires.pop().unwrap_or_else(|| WireNode {
                        tag: "view".into(),
                        ..Default::default()
                    });
                    let result = context
                        .tree
                        .borrow_mut()
                        .update_slots(wire, &prepared.slots);
                    if let Err(error) = result {
                        context.report(&[], "mount", error);
                        // The validated fallback has no authored bindings or classes.
                        if let Err(error) = context.tree.borrow_mut().update(WireNode {
                            tag: "view".into(),
                            ..Default::default()
                        }) {
                            context.report(&[], "mount", error);
                        }
                        prepared = Prepared::new(&context);
                    }
                    if let Some(root) = context.tree.borrow().root.clone() {
                        prepared.attach(&[root], &context);
                    }
                    prepared.registrations
                })
            })
        });
        registrations.reactive.push(owner);
        Self {
            registrations: Some(registrations),
            scope,
            context,
            routes: RefCell::new(vec![]),
            patch_passes: Cell::new(0),
        }
    }
    pub fn tree(&self) -> ViewTree {
        let mut tree = self
            .context
            .tree
            .borrow()
            .snapshot()
            .unwrap_or_else(|| Node {
                id: "view/empty".into(),
                style: Default::default(),
                text: None,
                on_click: None,
                children: vec![],
            });
        fn routes(node: &mut Node, tokens: &mut Vec<usize>) {
            if let Some(token) = node.on_click {
                node.on_click = Some(tokens.len());
                tokens.push(token);
            }
            for child in &mut node.children {
                routes(child, tokens);
            }
        }
        let mut tokens = vec![];
        routes(&mut tree, &mut tokens);
        *self.routes.borrow_mut() = tokens;
        self.context.pending_frame.set(false);
        ViewTree::new(tree, &self.context)
    }
    pub fn dispatch(&self, handler: usize) -> bool {
        let token = self.routes.borrow().get(handler).copied();
        token.is_some_and(|token| self.invoke(token, Event::Click))
    }
    /// Typed dispatch used by platform event adapters.
    pub fn dispatch_to(&self, node_id: &str, event: Event) -> bool {
        let token = self
            .context
            .listeners
            .borrow()
            .iter()
            .find_map(|(token, listener)| {
                (listener.kind == event.kind()
                    && listener
                        .anchor
                        .borrow()
                        .as_ref()
                        .is_some_and(|node| node.renderer_id() == node_id))
                .then_some(*token)
            });
        token.is_some_and(|token| self.invoke(token, event))
    }
    fn invoke(&self, token: usize, event: Event) -> bool {
        let callback = self
            .context
            .listeners
            .borrow()
            .get(&token)
            .map(|listener| listener.callback.clone());
        let Some(callback) = callback else {
            return false;
        };
        let before = self.context.patches.get();
        let path = self
            .context
            .listeners
            .borrow()
            .get(&token)
            .and_then(|listener| listener.anchor.borrow().as_ref().map(NodeHandle::slot))
            .unwrap_or_default();
        self.context
            .owned_event(&self.scope, &path, "event", || callback.borrow_mut()(event));
        if self.context.patches.get() != before {
            self.patch_passes.set(self.patch_passes.get() + 1);
        }
        true
    }
    /// Drain the last 128 operational mount/binding errors (oldest evicted).
    /// Every occurrence has already been delivered to the sink.
    /// Drain operational mount/binding errors. Failed patches retain the last
    /// valid tree; other reactions continue in the same flush.
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.context.errors.borrow_mut())
    }
    pub fn patch_passes(&self) -> usize {
        self.patch_passes.get()
    }
    /// Number of retained-property patches (diagnostic, not renderer frames).
    pub fn property_patches(&self) -> usize {
        self.context.patches.get()
    }
}
impl Drop for UiApp {
    fn drop(&mut self) {
        self.context.at(&[], "drop", || {
            self.scope.run(|| {
                drop(self.registrations.take());
                self.context.event_owners.borrow_mut().clear();
            })
        });
        self.context.waker.borrow_mut().take();
        crate::retained::unregister(self.context.session_id);
    }
}
impl Application for UiApp {
    fn initial_state(&self) -> Vec<f64> {
        vec![]
    }
    fn render(&self, _: &[f64]) -> Node {
        self.tree().snapshot()
    }
    fn event(&self, handler: usize, _: &mut [f64]) {
        self.dispatch(handler);
    }
    fn semantics(&self) -> Vec<deka_native_ui::SemanticNode> {
        use deka_native_ui::{SemanticNode, SemanticRole};
        fn visit(
            node: &NodeHandle,
            parent: Option<String>,
            hidden: bool,
            disabled: bool,
            output: &mut Vec<SemanticNode>,
        ) {
            let tag = node.tag_name();
            let clickable = node.has_click_handler();
            let role = match tag.as_deref() {
                Some("button") => SemanticRole::Button,
                Some("input") => SemanticRole::TextInput,
                Some("textarea") => SemanticRole::MultilineTextInput,
                None => SemanticRole::Label,
                _ if clickable => SemanticRole::Button,
                _ if node.all_children().is_empty() && !node.text_content().is_empty() => {
                    SemanticRole::Label
                }
                _ => SemanticRole::Group,
            };
            let hidden = hidden || node.attribute("aria-hidden").as_deref() == Some("true");
            let disabled = disabled
                || node
                    .attribute("disabled")
                    .is_some_and(|v| !matches!(v.as_str(), "false" | "0"));
            let natural = matches!(
                role,
                SemanticRole::Button | SemanticRole::TextInput | SemanticRole::MultilineTextInput
            ) || clickable;
            let name = node.attribute("aria-label").unwrap_or_else(|| {
                if matches!(
                    role,
                    SemanticRole::TextInput | SemanticRole::MultilineTextInput
                ) {
                    node.attribute("placeholder").unwrap_or_default()
                } else if natural || role == SemanticRole::Label {
                    node.text_content()
                } else {
                    String::new()
                }
            });
            let id = node.renderer_id();
            output.push(SemanticNode {
                id: id.clone(),
                parent,
                role,
                name,
                value: node.attribute("value").unwrap_or_default(),
                disabled,
                hidden,
                tab_index: node
                    .attribute("tabIndex")
                    .and_then(|v| v.parse().ok())
                    .or(natural.then_some(0)),
                clickable,
            });
            // A button's descendant text supplies its accessible name.
            if role != SemanticRole::Button {
                for child in node.all_children() {
                    visit(&child, Some(id.clone()), hidden, disabled, output);
                }
            }
        }
        let mut output = vec![];
        if let Some(root) = self.context.tree.borrow().root.as_ref() {
            visit(root, None, false, false, &mut output);
        }
        output
    }
    fn report_error(&self, operation: &str, message: String) {
        self.context.report(&[], operation, message);
    }
    fn context_menu(&self, id: &str, x: f32, y: f32) -> bool {
        if self
            .semantics()
            .iter()
            .any(|node| node.id == id && (node.disabled || node.hidden))
        {
            return false;
        }
        fn find(node: &NodeHandle, id: &str) -> Option<NodeHandle> {
            if node.renderer_id() == id {
                return Some(node.clone());
            }
            node.all_children().iter().find_map(|node| find(node, id))
        }
        let mut node = self
            .context
            .tree
            .borrow()
            .root
            .as_ref()
            .and_then(|node| find(node, id));
        while let Some(current) = node {
            if current
                .attribute("disabled")
                .is_some_and(|value| !matches!(value.as_str(), "false" | "0"))
                || current.attribute("aria-hidden").as_deref() == Some("true")
            {
                return false;
            }
            if self.dispatch_to(&current.renderer_id(), Event::ContextMenu { x, y }) {
                return true;
            }
            node = current.parent();
        }
        false
    }
    fn text_controls(&self) -> Vec<deka_native_ui::TextControl> {
        let revision = self.context.patches.get();
        let mut index = self.context.text_index.borrow_mut();
        if index.as_ref().is_none_or(|(cached, _)| *cached != revision) {
            let controlled: std::collections::BTreeSet<_> = self
                .context
                .listeners
                .borrow()
                .values()
                .filter(|l| l.kind == EventKind::Input && l.controlled_value)
                .filter_map(|l| l.anchor.borrow().as_ref().map(NodeHandle::renderer_id))
                .collect();
            fn visit(
                node: &NodeHandle,
                controlled: &std::collections::BTreeSet<String>,
                out: &mut Vec<(NodeHandle, bool)>,
            ) {
                if matches!(node.tag_name().as_deref(), Some("input" | "textarea")) {
                    out.push((node.clone(), controlled.contains(&node.renderer_id())));
                }
                for child in node.all_children() {
                    visit(&child, controlled, out);
                }
            }
            let mut nodes = vec![];
            if let Some(root) = self.context.tree.borrow().root.as_ref() {
                visit(root, &controlled, &mut nodes);
            }
            *index = Some((revision, nodes));
        }
        index
            .as_ref()
            .unwrap()
            .1
            .iter()
            .map(|(node, controlled)| deka_native_ui::TextControl {
                id: node.renderer_id(),
                value: node.attribute("value").unwrap_or_default(),
                placeholder: node.attribute("placeholder").unwrap_or_default(),
                controlled: *controlled,
                multiline: node.tag_name().as_deref() == Some("textarea"),
            })
            .collect()
    }

    fn text_input(&self, id: &str, value: String) -> bool {
        fn find(node: &NodeHandle, id: &str) -> Option<NodeHandle> {
            if node.renderer_id() == id {
                return Some(node.clone());
            }
            node.all_children().iter().find_map(|node| find(node, id))
        }
        let node = self
            .context
            .tree
            .borrow()
            .root
            .as_ref()
            .and_then(|node| find(node, id));
        let Some(node) =
            node.filter(|node| matches!(node.tag_name().as_deref(), Some("input" | "textarea")))
        else {
            return false;
        };
        if node.attribute("value").unwrap_or_default() == value {
            return false;
        }
        let changed = match node.set_attribute("value", value.clone()) {
            Ok(changed) => changed,
            Err(error) => {
                self.context.report(&node.slot(), "value", error);
                return false;
            }
        };
        self.context.node_changed(&node, changed);
        // Browsers send a final input after compositionend; an unchanged full
        // value is not another committed edit or user callback.
        if !changed {
            return false;
        }
        self.dispatch_to(id, Event::Input(value));
        true
    }
    fn key_input(&self, id: &str, key: String) -> bool {
        self.dispatch_to(id, Event::KeyDown(key))
    }
    fn run_turn(&mut self, _budget: usize) -> bool {
        self.context.pending_frame.get()
    }
    fn set_waker(&mut self, waker: deka_native_ui::Waker) {
        if self.context.pending_frame.get() {
            waker.wake();
        }
        *self.context.waker.borrow_mut() = Some(waker);
    }
}
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
pub fn launch<A, M>(app: A)
where
    A: LaunchApp<M>,
{
    app.launch();
}
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub struct SingleWindow<M>(std::marker::PhantomData<M>);
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub struct MultipleWindows;
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub trait LaunchApp<M> {
    fn launch(self);
}
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
impl<A: BuildApp<M>, M> LaunchApp<SingleWindow<M>> for A {
    fn launch(self) {
        deka_native_ui::run(UiApp::new(self));
    }
}
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
impl LaunchApp<MultipleWindows> for crate::DesktopApp {
    fn launch(self) {
        deka_native_ui::window::run_windows(self);
    }
}

/// Launch with the existing window's explicit options and motion preference.
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
pub fn launch_with<A, M>(app: A, options: deka_native_ui::window::Options, reduced_motion: bool)
where
    A: BuildApp<M>,
{
    launch_with_options(
        app,
        LaunchOptions {
            window: options,
            reduced_motion,
            error_sink: None,
        },
    );
}

/// Launch with a host callback installed before any bindings are evaluated.
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
pub fn launch_with_options<A, M>(app: A, options: LaunchOptions)
where
    A: BuildApp<M>,
{
    launch_configured(app, options, deka_native_ui::window::run_with);
}
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
fn launch_configured<A, M, R>(
    app: A,
    options: LaunchOptions,
    host: impl FnOnce(UiApp, deka_native_ui::window::Options, bool) -> R,
) -> R
where
    A: BuildApp<M>,
{
    let app = UiApp::new_with_error_sink(app, options.error_sink);
    host(app, options.window, options.reduced_motion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal;

    #[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
    #[test]
    fn launch_option_sink_receives_running_binding_error() {
        let errors = Rc::new(RefCell::new(Vec::new()));
        let output = errors.clone();
        let mut options =
            LaunchOptions::new(deka_native_ui::window::Options::new("sink", 320., 240.));
        options.error_sink = Some(Rc::new(move |error| {
            output.borrow_mut().push(error.clone())
        }));
        launch_configured(
            || {
                let invalid = signal(false);
                View::element("view")
                    .attr("className", move || {
                        if invalid.get().unwrap() {
                            "p-bad"
                        } else {
                            "p-2"
                        }
                    })
                    .child(View::element("button").on_click(move |_| invalid.toggle()))
            },
            options,
            |app, _, _| {
                let mut state = app.initial_state();
                let before = app.render(&state);
                app.event(0, &mut state);
                assert_eq!(app.render(&state), before);
                assert_eq!(errors.borrow().len(), 1);
                assert_eq!(errors.borrow()[0].binding, "className");
            },
        );
    }
    #[test]
    fn host_application_binding_errors_reach_sink_once_with_location() {
        let errors = Rc::new(RefCell::new(Vec::new()));
        let output = errors.clone();
        let app = UiApp::new_with_error_sink(
            || {
                let invalid = signal(false);
                View::element("view")
                    .child(View::element("p").attr("className", move || {
                        if invalid.get().unwrap() {
                            "p-bad"
                        } else {
                            "p-2"
                        }
                    }))
                    .child(View::element("button").on_click(move |_| invalid.toggle()))
            },
            Some(Rc::new(move |error| {
                output.borrow_mut().push(error.clone())
            })),
        );
        let mut state = app.initial_state();
        let before = app.render(&state);
        app.event(0, &mut state);
        assert_eq!(app.render(&state), before);
        app.render(&state);
        assert_eq!(errors.borrow().len(), 1);
        assert_eq!(errors.borrow()[0].path, [0]);
        assert_eq!(errors.borrow()[0].binding, "className");
        assert!(!errors.borrow()[0].message.is_empty());
        assert_eq!(app.take_errors().len(), 1);
        assert!(app.take_errors().is_empty());
        assert_eq!(errors.borrow().len(), 1);
    }
    #[test]
    fn sink_reports_initial_errors_and_diagnostic_buffer_is_bounded() {
        let calls = Rc::new(Cell::new(0));
        let output = calls.clone();
        let app = UiApp::new_with_error_sink(
            || View::element("view").attr("className", "p-bad"),
            Some(Rc::new(move |_| output.set(output.get() + 1))),
        );
        assert_eq!(calls.get(), 1);
        for n in 0..1000 {
            app.context.report(&[3], "test", n.to_string());
        }
        assert_eq!(calls.get(), 1001);
        let buffer = app.take_errors();
        assert_eq!(buffer.len(), ERROR_BUFFER_LIMIT);
        assert!(buffer[0].ends_with("872"));
        assert!(buffer.last().unwrap().ends_with("999"));
    }
    #[test]
    fn disposed_toggle_and_text_conveniences_render_defaults_and_report() {
        let errors = Rc::new(RefCell::new(Vec::new()));
        let output = errors.clone();
        let app = UiApp::new_with_error_sink(
            || {
                let ((toggle, computed, text), owner) = crate::reactive::owned(|| {
                    (
                        signal(true),
                        crate::derived(|| true),
                        signal("old".to_owned()),
                    )
                });
                drop(owner);
                View::element("view")
                    .class("p-4", toggle)
                    .class("opacity-25", computed)
                    .child(text)
            },
            Some(Rc::new(move |error| {
                output.borrow_mut().push(error.clone())
            })),
        );
        assert_eq!(errors.borrow().len(), 3);
        assert!(
            errors
                .borrow()
                .iter()
                .all(|error| error.message.contains("disposed"))
        );
        assert_eq!(errors.borrow()[0].binding, "className");
        assert_eq!(errors.borrow()[2].binding, "text");
        assert!(
            app.tree()
                .children
                .iter()
                .all(|node| node.text.as_deref() == Some(""))
        );
        assert_eq!(app.tree().style.opacity, 1.);
    }
    #[test]
    fn non_element_builder_misuse_is_reported_without_unwinding() {
        let app = UiApp::new(|| View::fragment([]).attr("id", "bad").child("bad"));
        let errors = app.take_errors();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("builder: attributes/events/children require an element"));
        let app = UiApp::new(|| View::dynamic(|| "child").on_click(|_| {}));
        assert_eq!(app.take_errors().len(), 1);
        let app = UiApp::new(|| View::text("child").attr("unsupported", "x"));
        assert_eq!(app.take_errors().len(), 1);
    }
    #[test]
    fn invalid_reactive_classes_report_without_losing_the_last_valid_tree() {
        let app = UiApp::new(|| {
            let invalid = signal(false);
            View::element("view")
                .child(
                    View::element("p")
                        .attr("id", "target")
                        .attr("className", move || {
                            if invalid.get().unwrap() {
                                "p-bad"
                            } else {
                                "p-2"
                            }
                        })
                        .child("Kept"),
                )
                .child(View::element("button").on_click(move |_| invalid.toggle()))
        });
        let before = app.tree();
        app.dispatch(0);
        assert_eq!(app.tree(), before);
        assert_eq!(app.take_errors().len(), 1);
        app.dispatch(0);
        assert_eq!(app.tree(), before);
        assert!(app.take_errors().is_empty());
        let initial = UiApp::new(|| View::element("view").attr("className", "p-bad"));
        assert_eq!(initial.take_errors().len(), 1);
        assert!(initial.tree().children.is_empty());
        let toggle = UiApp::new(|| View::element("view").class("p-bad", true));
        assert_eq!(toggle.take_errors().len(), 1);
    }

    #[test]
    fn failed_slot_replacement_reports_and_the_remaining_reactions_finish() {
        let app = UiApp::new(|| {
            let state = signal(false);
            View::element("view")
                .on_click(move |_| state.toggle())
                .child(move || {
                    if state.get().unwrap() {
                        View::text("New")
                    } else {
                        View::text("Old")
                    }
                })
                .child(View::live_text(move || state.get().unwrap()))
        });
        let before = app.tree();
        let mut foreign = Tree::default();
        foreign
            .update(WireNode {
                tag: "view".into(),
                ..Default::default()
            })
            .unwrap();
        let anchor = app
            .context
            .listeners
            .borrow()
            .values()
            .next()
            .unwrap()
            .anchor
            .clone();
        let original = anchor.replace(foreign.root.clone());
        app.dispatch(0);
        let after = app.tree();
        assert_eq!(after.children[0], before.children[0]);
        assert_eq!(after.children[1].text.as_deref(), Some("true"));
        assert_eq!(app.take_errors(), ["invalid retained structural slot"]);
        anchor.replace(original);
        app.dispatch(0);
        assert!(app.take_errors().is_empty());
        assert_eq!(app.tree().children[1].text.as_deref(), Some("false"));
        app.dispatch(0);
        assert_eq!(app.tree().children[0].text.as_deref(), Some("New"));
    }

    #[test]
    fn invalid_dynamic_classes_release_new_registrations_and_recover() {
        let app = UiApp::new(|| {
            let invalid = signal(false);
            View::element("view")
                .child(move || {
                    View::element("p")
                        .attr(
                            "className",
                            if invalid.get().unwrap() {
                                "p-bad"
                            } else {
                                "p-2"
                            },
                        )
                        .child("Kept")
                })
                .child(View::element("button").on_click(move |_| invalid.toggle()))
        });
        let before = app.tree();
        app.dispatch(0);
        assert_eq!(app.tree(), before);
        assert_eq!(app.take_errors().len(), 1);
        app.dispatch(0);
        assert_eq!(app.tree(), before);
        assert!(app.take_errors().is_empty());
    }

    #[test]
    fn imperative_edits_survive_unchanged_authored_values_and_other_property_patches() {
        let app = UiApp::new(|| {
            let mut count = signal(0);
            let red = signal(false);
            View::element("view")
                .child(
                    View::element("p")
                        .attr("id", "message")
                        .attr("className", move || {
                            if red.get().unwrap() {
                                "text-[#ff0000]"
                            } else {
                                "text-[#0000ff]"
                            }
                        })
                        .child(View::live_text(move || count.get().unwrap() % 2)),
                )
                .child(
                    View::element("button")
                        .on_click(move |_| count += 2)
                        .child("Same value"),
                )
                .child(
                    View::element("button")
                        .on_click(move |_| count += 1)
                        .child("Different text"),
                )
                .child(
                    View::element("button")
                        .on_click(move |_| red.toggle())
                        .child("Different class"),
                )
        });
        let paragraph = app.context.tree.borrow().element_by_id("message").unwrap();
        let leaf = paragraph.all_children()[0].clone();
        leaf.0.borrow_mut().text = Some("Hand edit".into());
        paragraph.0.borrow_mut().text = Some("Element edit".into());
        paragraph.0.borrow_mut().style.opacity = 0.25;
        let before = app.tree();
        app.dispatch(0);
        assert_eq!(app.tree(), before);
        assert_eq!(app.property_patches(), 0);
        app.dispatch(1);
        let changed = app.tree();
        assert_eq!(changed.children[0].children[0].text.as_deref(), Some("1"));
        assert_eq!(changed.children[0].style.opacity, 0.25);
        app.dispatch(2);
        let changed = app.tree();
        assert_eq!(changed.children[0].children[0].text.as_deref(), Some("1"));
        assert_eq!(changed.children[0].style.opacity, 1.);
        assert_eq!(changed.children[0].style.color, Some(0xff0000));
        assert!(changed.children[0].style.row);
        assert_eq!(changed.children[0].text.as_deref(), Some("Element edit"));
        assert_eq!(changed.children[1..], before.children[1..]);
    }

    #[test]
    fn static_and_live_attributes_are_set_once_and_keep_authored_ownership() {
        let evaluations = Rc::new(Cell::new(0));
        let seen = evaluations.clone();
        let app = UiApp::new(move || {
            let mut value = signal(0);
            View::element("view")
                .child(
                    View::element("p")
                        .attr("id", "message")
                        .attr("placeholder", move || {
                            seen.set(seen.get() + 1);
                            value.get().unwrap() % 2
                        })
                        .child("Static"),
                )
                .child(
                    View::element("button")
                        .on_click(move |_| value += 2)
                        .child("Same"),
                )
                .child(
                    View::element("button")
                        .on_click(move |_| value += 1)
                        .child("Different"),
                )
        });
        let node = app.context.tree.borrow().element_by_id("message").unwrap();
        assert_eq!(node.attribute("placeholder").as_deref(), Some("0"));
        assert_eq!(evaluations.get(), 1);
        // A direct effective edit is a resource bridge operation, not an author
        // write. Preserve it across idle frames and unchanged author results.
        node.0.borrow_mut().text = Some("Hand edit".into());
        let before = app.tree();
        for _ in 0..60 {
            app.tree();
        }
        assert_eq!(evaluations.get(), 1);
        app.dispatch(0);
        assert_eq!(app.tree(), before);
        assert_eq!(node.attribute("placeholder").as_deref(), Some("0"));
        app.dispatch(1);
        assert_eq!(node.attribute("placeholder").as_deref(), Some("1"));
        assert_eq!(node.attribute("id").as_deref(), Some("message"));
        assert_eq!(node.snapshot().children, before.children[0].children);
        assert_eq!(node.snapshot().text.as_deref(), Some("Hand edit"));
    }

    #[test]
    fn replaced_structural_bindings_and_event_tables_remain_bounded() {
        let app = UiApp::new(|| {
            let visible = signal(false);
            View::element("view")
                .child(move || {
                    visible
                        .get()
                        .unwrap()
                        .then(|| View::element("button").on_click(|_| {}).child("Temporary"))
                })
                .child(
                    View::element("button")
                        .on_click(move |_| visible.toggle())
                        .child("Permanent"),
                )
        });
        for _ in 0..100 {
            app.tree();
            assert!(app.dispatch(0));
            app.tree();
            assert_eq!(app.context.listeners.borrow().len(), 2);
            assert!(app.dispatch(1));
            app.tree();
            assert_eq!(app.context.listeners.borrow().len(), 1);
        }
    }

    #[test]
    fn dynamic_class_base_and_multiple_toggles_coalesce_into_one_property_patch() {
        let app = UiApp::new(|| {
            let state = crate::signal(false);
            View::element("view")
                .child(
                    View::element("p")
                        .attr("className", move || {
                            if state.get().unwrap() { "p-4" } else { "p-2" }
                        })
                        .class("opacity-0", state)
                        .class("rounded", move || state.get().unwrap())
                        .child("Same"),
                )
                .child(
                    View::element("button")
                        .on_click(move |_| state.toggle())
                        .child("Toggle"),
                )
        });
        let before = app.tree();
        app.dispatch(0);
        let after = app.tree();
        assert_eq!(after.children[0].style.padding.top, 16.);
        assert_eq!(after.children[0].style.opacity, 0.);
        assert_eq!(after.children[0].style.radius, 4.);
        assert_eq!(after.children[0].children, before.children[0].children);
        assert_eq!(app.property_patches(), 1);
    }
}
