//! Typed access to the shared retained store (APS 73 and APS 74).
use crate::{reactive, view::Context};
use deka_native_ir::{Node, selectors::Selector, tree::NodeHandle};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fmt,
    ops::Deref,
    rc::{Rc, Weak},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewError {
    NoActiveApp,
    SessionDropped,
    ForeignSession,
    InvalidOperation(String),
}
impl fmt::Display for ViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoActiveApp => f.write_str("tree access requires an app scope"),
            Self::SessionDropped => f.write_str("tree session has been dropped"),
            Self::ForeignSession => f.write_str("node belongs to another tree session"),
            Self::InvalidOperation(error) => f.write_str(error),
        }
    }
}
impl std::error::Error for ViewError {}
impl From<String> for ViewError {
    fn from(error: String) -> Self {
        Self::InvalidOperation(error)
    }
}
thread_local! {
    static APPS: RefCell<BTreeMap<u64, Weak<Context>>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_SESSION: Cell<u64> = const { Cell::new(0) };
    static ACTIVE_TREE: RefCell<Option<Weak<Context>>> = const { RefCell::new(None) };
}
pub(crate) fn next_session_id() -> u64 {
    NEXT_SESSION.with(|next| {
        let id = next.get();
        next.set(id.checked_add(1).expect("tree session identity exhausted"));
        id
    })
}
pub(crate) fn register(context: &Rc<Context>) {
    APPS.with(|apps| {
        let mut apps = apps.borrow_mut();
        apps.retain(|_, app| app.strong_count() > 0);
        apps.insert(context.session_id, Rc::downgrade(context));
    });
}
pub(crate) fn unregister(session: u64) {
    APPS.with(|apps| {
        apps.borrow_mut().remove(&session);
    });
}
pub(crate) fn active_context() -> Option<Weak<Context>> {
    ACTIVE_TREE.with(|tree| tree.borrow().clone())
}
pub(crate) fn with_active<R>(context: Option<Weak<Context>>, run: impl FnOnce() -> R) -> R {
    struct Restore(Option<Weak<Context>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE_TREE.with(|tree| {
                tree.replace(self.0.take());
            });
        }
    }
    let previous = ACTIVE_TREE.with(|tree| tree.replace(context));
    let _restore = Restore(previous);
    run()
}
pub(crate) fn with_session<R>(session: u64, run: impl FnOnce() -> R) -> R {
    let context = APPS.with(|apps| apps.borrow().get(&session).cloned());
    with_active(context, run)
}
/// Access the originating window during construction, events and reactions.
pub fn tree() -> Result<ViewTree, ViewError> {
    let scope = reactive::current_scope_id().ok_or(ViewError::NoActiveApp)?;
    let context = if let Some(active) = active_context() {
        active
            .upgrade()
            .filter(|context| context.scope_id == scope)
            .ok_or(ViewError::SessionDropped)?
    } else {
        APPS.with(|apps| {
            let mut matching = apps
                .borrow()
                .values()
                .filter_map(Weak::upgrade)
                .filter(|context| context.scope_id == scope)
                .collect::<Vec<_>>();
            (matching.len() == 1).then(|| matching.pop().unwrap())
        })
        .ok_or(ViewError::NoActiveApp)?
    };
    Ok(ViewTree::new(context.snapshot(), &context))
}

/// An immutable presentation snapshot with live queries into its owning store.
/// Deref exposes the renderer Node; queries always observe current effective data.
#[derive(Clone)]
pub struct ViewTree {
    snapshot: Node,
    context: Weak<Context>,
}
impl ViewTree {
    pub(crate) fn new(snapshot: Node, context: &Rc<Context>) -> Self {
        Self {
            snapshot,
            context: Rc::downgrade(context),
        }
    }
    pub fn snapshot(&self) -> Node {
        self.snapshot.clone()
    }
    pub fn query(&self, selector: &str) -> Result<Option<ViewElement>, ViewError> {
        let selector = Selector::parse(selector).map_err(ViewError::from)?;
        let context = self.context.upgrade().ok_or(ViewError::SessionDropped)?;
        let nodes = context.tree.borrow().query(&selector, false);
        Ok(nodes
            .into_iter()
            .next()
            .map(|node| ViewElement(ViewNode::new(node, &context))))
    }
    pub fn query_all(&self, selector: &str) -> Result<Vec<ViewElement>, ViewError> {
        let selector = Selector::parse(selector).map_err(ViewError::from)?;
        let context = self.context.upgrade().ok_or(ViewError::SessionDropped)?;
        let nodes = context.tree.borrow().query(&selector, true);
        Ok(nodes
            .into_iter()
            .map(|node| ViewElement(ViewNode::new(node, &context)))
            .collect())
    }
    pub fn get_element_by_id(&self, id: &str) -> Option<ViewElement> {
        let context = self.context.upgrade()?;
        let node = context.tree.borrow().element_by_id(id)?;
        Some(ViewElement(ViewNode::new(node, &context)))
    }
    /// Validate a handle before handing it to an integration for this session.
    pub fn check_node(&self, node: &ViewNode) -> Result<(), ViewError> {
        let context = self.context.upgrade().ok_or(ViewError::SessionDropped)?;
        if context.tree.borrow().owns(&node.node) {
            Ok(())
        } else {
            Err(ViewError::ForeignSession)
        }
    }
}
impl Deref for ViewTree {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.snapshot
    }
}
impl PartialEq for ViewTree {
    fn eq(&self, other: &Self) -> bool {
        self.snapshot == other.snapshot
    }
}
impl fmt::Debug for ViewTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.snapshot.fmt(f)
    }
}

/// An opaque live node, including text nodes. It owns its allocation even after
/// detachment or app teardown; it never keeps the app or reactive scope alive.
#[derive(Clone)]
pub struct ViewNode {
    pub(crate) node: NodeHandle,
    context: Weak<Context>,
}
impl ViewNode {
    fn new(node: NodeHandle, context: &Rc<Context>) -> Self {
        Self {
            node,
            context: Rc::downgrade(context),
        }
    }
    pub fn as_element(&self) -> Option<ViewElement> {
        self.node.is_element().then(|| ViewElement(self.clone()))
    }
    pub fn parent_node(&self) -> Option<Self> {
        Some(Self {
            node: self.node.parent()?,
            context: self.context.clone(),
        })
    }
    pub fn child_nodes(&self) -> Vec<Self> {
        self.node
            .all_children()
            .into_iter()
            .map(|node| Self {
                node,
                context: self.context.clone(),
            })
            .collect()
    }
    pub fn text_content(&self) -> String {
        self.node.text_content()
    }
    /// Detached handles may still be edited. Such edits affect only their
    /// retained allocation and never dirty or wake an application frame (APS 73).
    pub fn set_text_content(&self, text: impl Into<String>) -> Result<(), ViewError> {
        let changed = self.node.set_text_content(text.into());
        self.changed(changed);
        Ok(())
    }
    pub fn component_state<T: Clone + 'static>(&self) -> Option<T> {
        let context = self.context.upgrade()?;
        let states = context.states.borrow();
        let mut current = Some(self.node.clone());
        while let Some(node) = current {
            for (_, (anchor, value)) in states.iter().rev() {
                if anchor.borrow().as_ref().is_some_and(|owner| owner == &node)
                    && let Some(state) = value.downcast_ref::<T>()
                {
                    return Some(state.clone());
                }
            }
            current = node.parent();
        }
        None
    }
    fn changed(&self, changed: bool) {
        if let Some(context) = self.context.upgrade() {
            context.node_changed(&self.node, changed);
        }
    }
}
impl PartialEq for ViewNode {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
    }
}
impl Eq for ViewNode {}
impl fmt::Debug for ViewNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ViewNode")
            .field(&self.node.snapshot().id)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewElement(ViewNode);
impl Deref for ViewElement {
    type Target = ViewNode;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl ViewElement {
    pub fn children(&self) -> Vec<Self> {
        self.0
            .child_nodes()
            .into_iter()
            .filter_map(|node| node.as_element())
            .collect()
    }
    pub fn tag_name(&self) -> Option<String> {
        self.0.node.tag_name()
    }
    pub fn get_attribute(&self, name: &str) -> Option<String> {
        self.0.node.attribute(name)
    }
    pub fn set_attribute(&self, name: &str, value: impl Into<String>) -> Result<(), ViewError> {
        let changed = self
            .0
            .node
            .set_attribute(name, value.into())
            .map_err(ViewError::from)?;
        self.0.changed(changed);
        Ok(())
    }
    pub fn class_list(&self) -> ViewClassList {
        ViewClassList(self.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewClassList(ViewElement);
impl ViewClassList {
    pub fn tokens(&self) -> Vec<String> {
        self.0
            .get_attribute("className")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
    pub fn contains(&self, token: &str) -> bool {
        self.tokens().iter().any(|value| value == token)
    }
    fn validate(token: &str) -> Result<(), ViewError> {
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(ViewError::InvalidOperation(
                "class operation requires one utility".into(),
            ));
        }
        deka_native_ir::apply_classes(&mut Default::default(), token).map_err(ViewError::from)
    }
    pub fn add(&self, token: &str) -> Result<(), ViewError> {
        Self::validate(token)?;
        let mut tokens = self.tokens();
        if self.contains(token) {
            return Ok(());
        }
        tokens.push(token.into());
        self.0.set_attribute("className", tokens.join(" "))
    }
    pub fn remove(&self, token: &str) -> Result<(), ViewError> {
        Self::validate(token)?;
        let mut tokens = self.tokens();
        if !tokens.iter().any(|value| value == token) {
            return Ok(());
        }
        tokens.retain(|value| value != token);
        self.0.set_attribute("className", tokens.join(" "))
    }
    pub fn toggle(&self, token: &str) -> Result<bool, ViewError> {
        if self.contains(token) {
            self.remove(token)?;
            Ok(false)
        } else {
            self.add(token)?;
            Ok(true)
        }
    }
}

#[derive(Default)]
struct RefData {
    node: RefCell<Option<ViewNode>>,
    owner: Cell<Option<u64>>,
}
#[derive(Clone, Default)]
pub struct NodeRef(Rc<RefData>);
pub fn node_ref() -> NodeRef {
    NodeRef::default()
}
impl NodeRef {
    /// Resolve only a node currently attached to the live application tree.
    /// A previously copied ViewNode retains its allocation after detachment.
    pub fn get(&self) -> Option<ViewNode> {
        let node = self.0.node.borrow().clone()?;
        let context = node.context.upgrade()?;
        context.is_attached(&node.node).then_some(node)
    }
    pub(crate) fn check(&self, session: u64) -> Result<(), String> {
        if self.0.owner.get().is_some_and(|owner| owner != session) {
            Err("node ref belongs to another tree session".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn capture(&self, node: NodeHandle, context: &Rc<Context>) {
        self.0.owner.set(Some(context.session_id));
        self.0.node.replace(Some(ViewNode::new(node, context)));
    }
}

/// Explicitly publishes typed Rust component state for retained-tree lookup.
/// A state struct may hold Signal handles; unpublished locals stay ordinary Rust.
pub struct ComponentState<T: 'static>(T);
impl<T: 'static> ComponentState<T> {
    pub fn new(state: T) -> Self {
        Self(state)
    }
    pub(crate) fn erase(self) -> Rc<dyn Any> {
        Rc::new(self.0)
    }
}
