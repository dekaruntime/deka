//! Transient Rust builders and reactive bindings on the shared native store.
use crate::{Derived, Effect, Scope, Signal, effect};
use deka_native_ir::{
    Node, WireNode,
    tree::{NodeHandle, Tree},
};
use deka_native_ui::Application;
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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Click,
    Input(String),
    KeyDown(String),
}
impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Self::Click => EventKind::Click,
            Self::Input(_) => EventKind::Input,
            Self::KeyDown(_) => EventKind::KeyDown,
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
}
struct Element {
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
    pub fn from_child<V, M>(value: V) -> Self
    where
        V: IntoView<M>,
    {
        value.into_view()
    }
    pub fn element(tag: impl Into<String>) -> Self {
        Self(Builder::Element(Element {
            tag: tag.into(),
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
    pub fn attr<V, M>(mut self, name: &str, value: V) -> Self
    where
        V: IntoAttribute<M>,
    {
        assert!(
            matches!(name, "id" | "className" | "value" | "placeholder"),
            "unsupported native attribute {name}"
        );
        self.element_mut()
            .attributes
            .insert(name.into(), value.into_attribute());
        self
    }
    pub fn class<V, M>(mut self, name: &str, value: V) -> Self
    where
        V: IntoToggle<M>,
    {
        assert!(
            !name.is_empty() && !name.chars().any(char::is_whitespace),
            "class toggle requires one utility"
        );
        deka_native_ir::apply_classes(&mut Default::default(), name)
            .expect("invalid class toggle utility");
        self.element_mut()
            .toggles
            .insert(name.into(), value.into_toggle());
        self
    }
    pub fn on(mut self, kind: EventKind, event: impl FnMut(Event) + 'static) -> Self {
        self.element_mut().events.insert(kind, Box::new(event));
        self
    }
    pub fn on_click(self, event: impl FnMut(Event) + 'static) -> Self {
        self.on(EventKind::Click, event)
    }
    pub fn child<V, M>(mut self, child: V) -> Self
    where
        V: IntoView<M>,
    {
        self.element_mut().children.push(child.into_view());
        self
    }
    pub fn children(mut self, children: impl IntoIterator<Item = View>) -> Self {
        self.element_mut().children.push(Self::fragment(children));
        self
    }
    fn element_mut(&mut self) -> &mut Element {
        match &mut self.0 {
            Builder::Element(element) => element,
            _ => panic!("attributes/events/children require an element"),
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
impl<T: Clone + Default + Display + 'static> IntoView<Tracked> for Signal<T> {
    fn into_view(self) -> View {
        View::live_text(move || self.get())
    }
}
impl<T: Clone + Default + Display + 'static> IntoView<Tracked> for Derived<T> {
    fn into_view(self) -> View {
        View::live_text(move || self.get())
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
        Box::new(move || self.get())
    }
}
impl IntoToggle<Tracked> for Derived<bool> {
    fn into_toggle(self) -> Box<dyn FnMut() -> bool> {
        Box::new(move || self.get())
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

type Anchor = Rc<RefCell<Option<NodeHandle>>>;
type Callback = Rc<RefCell<Box<dyn FnMut(Event)>>>;
struct Listener {
    anchor: Anchor,
    kind: EventKind,
    callback: Callback,
}
#[derive(Default)]
struct Context {
    tree: RefCell<Tree>,
    listeners: RefCell<BTreeMap<usize, Listener>>,
    next_listener: Cell<usize>,
    patches: Cell<usize>,
    waker: RefCell<Option<deka_native_ui::Waker>>,
}
impl Context {
    fn changed(&self, changed: bool) {
        if changed {
            self.patches.set(self.patches.get() + 1);
        }
    }
    fn listen(&self, anchor: Anchor, kind: EventKind, event: Box<dyn FnMut(Event)>) -> usize {
        let token = self.next_listener.get();
        self.next_listener
            .set(token.checked_add(1).expect("event identity exhausted"));
        self.listeners.borrow_mut().insert(
            token,
            Listener {
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
    reactive: Vec<crate::reactive::ReactiveOwner>,
    events: Vec<usize>,
    groups: Vec<Rc<RefCell<Registrations>>>,
    context: Weak<Context>,
}
impl Registrations {
    fn extend(&mut self, mut other: Self) {
        self.effects.append(&mut other.effects);
        self.reactive.append(&mut other.reactive);
        self.events.append(&mut other.events);
        self.groups.append(&mut other.groups);
    }
}
impl Drop for Registrations {
    fn drop(&mut self) {
        for effect in self.effects.drain(..) {
            effect.dispose();
        }
        if let Some(context) = self.context.upgrade() {
            for token in self.events.drain(..) {
                context.listeners.borrow_mut().remove(&token);
            }
        }
    }
}
struct Prepared {
    wires: Vec<WireNode>,
    slots: Vec<Vec<usize>>,
    anchors: Vec<(Vec<usize>, Anchor)>,
    registrations: Registrations,
}
impl Prepared {
    fn new(context: &Rc<Context>) -> Self {
        Self {
            wires: vec![],
            slots: vec![],
            anchors: vec![],
            registrations: Registrations {
                context: Rc::downgrade(context),
                effects: vec![],
                reactive: vec![],
                events: vec![],
                groups: vec![],
            },
        }
    }
    fn extend(&mut self, mut child: Self) {
        self.wires.append(&mut child.wires);
        self.slots.append(&mut child.slots);
        self.anchors.append(&mut child.anchors);
        self.registrations.extend(child.registrations);
    }
    fn attach(&self, roots: &[NodeHandle]) {
        fn visit(node: &NodeHandle, nodes: &mut BTreeMap<Vec<usize>, NodeHandle>) {
            nodes.insert(node.slot(), node.clone());
            for child in node.all_children() {
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
    }
}

fn bind(
    value: Attribute,
    anchor: &Anchor,
    context: &Rc<Context>,
    registrations: &mut Registrations,
    patch: impl Fn(&NodeHandle, String) -> bool + 'static,
) -> String {
    match value {
        Attribute::Value(value) => value,
        Attribute::Binding(mut getter) => {
            let initial = Rc::new(RefCell::new(String::new()));
            let output = initial.clone();
            let target = anchor.clone();
            let context = Rc::downgrade(context);
            registrations.effects.push(effect(move || {
                let value = getter();
                if let Some(node) = target.borrow().as_ref() {
                    if let Some(context) = context.upgrade() {
                        context.changed(patch(node, value));
                    }
                } else {
                    *output.borrow_mut() = value;
                }
            }));
            initial.take()
        }
    }
}
fn prepare(view: View, path: Vec<usize>, parent: Anchor, context: &Rc<Context>) -> Prepared {
    let mut prepared = Prepared::new(context);
    match view.0 {
        Builder::Fragment(children) => {
            for (index, child) in children.into_iter().enumerate() {
                let mut slot = path.clone();
                slot.push(index);
                prepared.extend(prepare(child, slot, parent.clone(), context));
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
                let (mut next, reactive) =
                    crate::reactive::owned(|| prepare(getter(), slot, parent.clone(), &context));
                next.registrations.reactive.push(reactive);
                let target = parent.borrow().clone();
                if let Some(parent) = target {
                    let wires = std::mem::take(&mut next.wires);
                    let roots = context
                        .tree
                        .borrow_mut()
                        .replace_slot(&parent, &prefix, wires, &next.slots)
                        .expect("invalid dynamic native children");
                    next.attach(&roots);
                    *owner.borrow_mut() = next.registrations;
                    context.changed(true);
                } else {
                    *output.borrow_mut() = Some(next);
                }
            }));
            let mut next = initial
                .borrow_mut()
                .take()
                .expect("initial dynamic children");
            *group.borrow_mut() = next.registrations;
            next.registrations = Registrations::default();
            prepared.registrations.groups.push(group);
            prepared.extend(next);
        }
        Builder::Text(value) => {
            let anchor = Anchor::default();
            let text = bind(
                value,
                &anchor,
                context,
                &mut prepared.registrations,
                NodeHandle::patch_text,
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
                &anchor,
                context,
                &mut prepared.registrations,
                NodeHandle::patch_classes,
            );
            if has_classes {
                wire.attributes
                    .insert("className".into(), wire.classes.clone());
            }
            for (name, value) in element.attributes {
                let property = name.clone();
                let value = bind(
                    value,
                    &anchor,
                    context,
                    &mut prepared.registrations,
                    move |node, value| node.patch_attribute(&property, value),
                );
                wire.attributes.insert(name, value);
            }
            for (kind, event) in element.events {
                let token = context.listen(anchor.clone(), kind, event);
                prepared.registrations.events.push(token);
                if kind == EventKind::Click {
                    wire.handler = Some(token);
                }
            }
            wire.style().expect("invalid native element or classes");
            prepared.slots.push(path.clone());
            prepared.anchors.push((path.clone(), anchor.clone()));
            for (index, child) in element.children.into_iter().enumerate() {
                let mut slot = path.clone();
                slot.push(index);
                let mut child = prepare(child, slot, anchor.clone(), context);
                wire.children.append(&mut child.wires);
                prepared.extend(child);
            }
            prepared.wires.push(wire);
        }
    }
    prepared
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
        let scope = Scope::new();
        let context = Rc::new(Context::default());
        let registrations = scope.run(|| {
            let view = app.build();
            let view = match view.0 {
                Builder::Element(_) | Builder::Text(_) => view,
                _ => View::element("view").child(view),
            };
            let mut prepared = prepare(view, vec![], Anchor::default(), &context);
            assert_eq!(prepared.wires.len(), 1, "app requires one root");
            context
                .tree
                .borrow_mut()
                .update_slots(prepared.wires.pop().unwrap(), &prepared.slots)
                .expect("invalid initial native tree");
            let root = context.tree.borrow().root.clone().unwrap();
            prepared.attach(&[root]);
            prepared.registrations
        });
        Self {
            registrations: Some(registrations),
            scope,
            context,
            routes: RefCell::new(vec![]),
            patch_passes: Cell::new(0),
        }
    }
    pub fn tree(&self) -> Node {
        let mut tree = self
            .context
            .tree
            .borrow()
            .snapshot()
            .expect("mounted native tree");
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
        tree
    }
    pub fn dispatch(&self, handler: usize) -> bool {
        let token = self.routes.borrow().get(handler).copied();
        token.is_some_and(|token| self.invoke(token, Event::Click))
    }
    /// Typed dispatch for integrations supplying input/key events. The current
    /// desktop Application seam supplies clicks; platform text editing belongs to the editor lane.
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
                        .is_some_and(|node| node.snapshot().id == node_id))
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
        self.scope.batch(|| callback.borrow_mut()(event));
        if self.context.patches.get() != before {
            self.patch_passes.set(self.patch_passes.get() + 1);
            if let Some(waker) = self.context.waker.borrow().as_ref() {
                waker.wake();
            }
        }
        true
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
        self.scope.run(|| drop(self.registrations.take()));
    }
}
impl Application for UiApp {
    fn initial_state(&self) -> Vec<f64> {
        vec![]
    }
    fn render(&self, _: &[f64]) -> Node {
        self.tree()
    }
    fn event(&self, handler: usize, _: &mut [f64]) {
        self.dispatch(handler);
    }
    fn set_waker(&mut self, waker: deka_native_ui::Waker) {
        *self.context.waker.borrow_mut() = Some(waker);
    }
}
#[cfg(feature = "desktop")]
pub fn launch<A, M>(app: A)
where
    A: BuildApp<M>,
{
    deka_native_ui::run(UiApp::new(app));
}

/// Launch with the existing window's explicit options and motion preference.
#[cfg(feature = "desktop")]
pub fn launch_with<A, M>(app: A, options: deka_native_ui::window::Options, reduced_motion: bool)
where
    A: BuildApp<M>,
{
    deka_native_ui::window::run_with(UiApp::new(app), options, reduced_motion);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal;

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
                            if red.get() {
                                "text-[#ff0000]"
                            } else {
                                "text-[#0000ff]"
                            }
                        })
                        .child(View::live_text(move || count.get() % 2)),
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
                            value.get() % 2
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
                        .attr("className", move || if state.get() { "p-4" } else { "p-2" })
                        .class("opacity-0", state)
                        .class("rounded", move || state.get())
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
