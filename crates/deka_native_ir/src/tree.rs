//! Rust-owned node identities live here; renderer Nodes are disposable snapshots.
//!
//! Structural slots retain static siblings when an earlier conditional is absent.
//! Dynamic list positions are not author keys; reordered lists are still positional.
use crate::{Node, Style, WireNode};
type Result<T> = std::result::Result<T, String>;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Identity(u64);

#[derive(Clone, PartialEq, Eq)]
enum Kind {
    Text,
    Element(String),
}

struct Prepared {
    kind: Kind,
    slot: Vec<usize>,
    classes: String,
    attributes: BTreeMap<String, String>,
    style: Style,
    text: Option<String>,
    handler: Option<usize>,
    children: Vec<Prepared>,
}
impl Prepared {
    fn new(wire: WireNode) -> Result<Self> {
        Self::with_slots(wire, &mut std::iter::repeat(vec![]))
    }
    fn with_slots(wire: WireNode, slots: &mut impl Iterator<Item = Vec<usize>>) -> Result<Self> {
        let slot = slots.next().ok_or("missing structural slot")?;
        let kind = if wire.text.is_some() {
            Kind::Text
        } else {
            Kind::Element(wire.tag.clone())
        };
        let style = wire.style()?;
        Ok(Self {
            kind,
            slot,
            classes: wire.classes,
            attributes: wire.attributes,
            style,
            text: wire.text,
            handler: wire.handler,
            children: wire
                .children
                .into_iter()
                .map(|child| Self::with_slots(child, slots))
                .collect::<Result<_>>()?,
        })
    }
}
pub struct Record {
    #[doc(hidden)]
    pub id: Identity,
    owner: Rc<Owner>,
    #[doc(hidden)]
    pub parent: Option<Weak<RefCell<Record>>>,
    authored_text: Option<String>,
    authored_classes: String,
    authored_attributes: BTreeMap<String, String>,
    attributes: BTreeMap<String, String>,
    kind: Kind,
    slot: Vec<usize>,
    #[doc(hidden)]
    pub classes: String,
    #[doc(hidden)]
    pub style: Style,
    #[doc(hidden)]
    pub text: Option<String>,
    handler: Option<usize>,
    #[doc(hidden)]
    pub children: Vec<NodeHandle>,
}

/// Owner tokens distinguish separate sessions even when both number their root 0.
#[derive(Default)]
struct Owner;
/// One allocation is one node. Children own their subtree; parent/index edges
/// are weak. The renderer receives snapshots, never these resource references.
#[derive(Clone)]
pub struct NodeHandle(pub Rc<RefCell<Record>>);
impl PartialEq for NodeHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for NodeHandle {}
impl NodeHandle {
    pub fn slot(&self) -> Vec<usize> {
        self.0.borrow().slot.clone()
    }
    pub fn all_children(&self) -> Vec<Self> {
        self.0.borrow().children.clone()
    }
    /// Patch one authored text property, preserving effective edits until its
    /// authored value changes. This is the same ownership rule used by retain.
    pub fn patch_text(&self, text: String) -> bool {
        self.0.borrow_mut().patch_text(Some(text))
    }
    pub fn patch_attribute(&self, name: &str, value: String) -> Result<bool> {
        if name == "className" {
            return self.patch_classes(value);
        }
        let mut record = self.0.borrow_mut();
        if record.authored_attributes.get(name) == Some(&value) {
            return Ok(false);
        }
        record
            .authored_attributes
            .insert(name.into(), value.clone());
        record.attributes.insert(name.into(), value);
        Ok(true)
    }
    pub fn patch_classes(&self, classes: String) -> Result<bool> {
        let mut record = self.0.borrow_mut();
        if record.authored_classes == classes {
            return Ok(false);
        }
        let wire = WireNode {
            tag: match &record.kind {
                Kind::Text => String::new(),
                Kind::Element(tag) => tag.clone(),
            },
            text: matches!(&record.kind, Kind::Text).then(String::new),
            classes: classes.clone(),
            ..Default::default()
        };
        let style = wire.style()?;
        record
            .authored_attributes
            .insert("className".into(), classes.clone());
        record
            .attributes
            .insert("className".into(), classes.clone());
        Ok(record.patch_classes(classes, style))
    }
    pub fn is_element(&self) -> bool {
        matches!(self.0.borrow().kind, Kind::Element(_))
    }
    pub fn parent(&self) -> Option<Self> {
        self.0.borrow().parent.as_ref()?.upgrade().map(Self)
    }
    pub fn children(&self) -> Vec<Self> {
        self.0
            .borrow()
            .children
            .iter()
            .filter(|node| node.is_element())
            .cloned()
            .collect()
    }
    pub fn text_content(&self) -> String {
        let record = self.0.borrow();
        record.text.clone().unwrap_or_default()
            + &record
                .children
                .iter()
                .map(Self::text_content)
                .collect::<String>()
    }
    pub fn attribute(&self, name: &str) -> Option<String> {
        let record = self.0.borrow();
        if name == "className" {
            record.attributes.get(name).map(|_| record.classes.clone())
        } else {
            record.attributes.get(name).cloned()
        }
    }
    pub fn element_by_id(&self, id: &str) -> Option<Self> {
        if self.is_element() && self.attribute("id").as_deref() == Some(id) {
            return Some(self.clone());
        }
        self.0
            .borrow()
            .children
            .iter()
            .find_map(|node| node.element_by_id(id))
    }
    pub fn snapshot(&self) -> Node {
        Self::snapshot_record(&self.0.borrow())
    }
    pub fn snapshot_record(record: &Record) -> Node {
        Node {
            id: format!("view/{}", record.id.0),
            style: record.style.clone(),
            text: record.text.clone(),
            on_click: record.handler,
            children: record.children.iter().map(Self::snapshot).collect(),
        }
    }
}
#[derive(Default)]
pub struct Tree {
    #[doc(hidden)]
    pub root: Option<NodeHandle>,
    #[doc(hidden)]
    pub records: BTreeMap<Identity, Weak<RefCell<Record>>>,
    next: u64,
    owner: Rc<Owner>,
}
impl super::selectors::SelectorTree for Tree {
    type Node = NodeHandle;
    fn with_element<R>(
        &self,
        node: &NodeHandle,
        read: impl for<'a> FnOnce(Option<super::selectors::Element<'a>>) -> R,
    ) -> R {
        let record = node.0.borrow();
        read(match &record.kind {
            Kind::Text => None,
            Kind::Element(tag) => Some(super::selectors::Element {
                tag,
                id: record.attributes.get("id").map(String::as_str),
                classes: &record.classes,
            }),
        })
    }
    fn parent(&self, node: &NodeHandle) -> Option<NodeHandle> {
        node.parent()
    }
    fn children(&self, node: &NodeHandle) -> Vec<NodeHandle> {
        node.0.borrow().children.clone()
    }
}
impl Tree {
    pub fn snapshot(&self) -> Option<Node> {
        self.root.as_ref().map(NodeHandle::snapshot)
    }
    /// Replace only a structural child slot using the existing retention path.
    /// Builders and wire values are transient; this store remains the sole tree.
    pub fn replace_slot(
        &mut self,
        parent: &NodeHandle,
        prefix: &[usize],
        wires: Vec<WireNode>,
        slots: &[Vec<usize>],
    ) -> Result<Vec<NodeHandle>> {
        if prefix.is_empty() || !self.owns(parent) {
            return Err("invalid retained structural slot".into());
        }
        let mut slots = slots.iter().cloned();
        let prepared = wires
            .into_iter()
            .map(|wire| Prepared::with_slots(wire, &mut slots))
            .collect::<Result<Vec<_>>>()?;
        if prepared.iter().any(|next| !next.slot.starts_with(prefix)) {
            return Err("child lies outside its structural slot".into());
        }
        let old = parent.all_children();
        let mut replacements = Vec::with_capacity(prepared.len());
        for next in prepared {
            let current = old.iter().find(|node| node.slot() == next.slot).cloned();
            let node = self.retain(current, next)?;
            node.0.borrow_mut().parent = Some(Rc::downgrade(&parent.0));
            replacements.push(node);
        }
        let mut children: Vec<_> = old
            .iter()
            .filter(|node| !node.slot().starts_with(prefix))
            .cloned()
            .collect();
        children.extend(replacements.iter().cloned());
        children.sort_by_key(NodeHandle::slot);
        for node in old {
            if !children.contains(&node) {
                node.0.borrow_mut().parent = None;
            }
        }
        parent.0.borrow_mut().children = children;
        self.records.retain(|_, record| record.strong_count() > 0);
        Ok(replacements)
    }
    pub fn query(&self, selector: &super::selectors::Selector, all: bool) -> Vec<NodeHandle> {
        let Some(root) = &self.root else {
            return vec![];
        };
        if all {
            selector.query_all(self, root)
        } else {
            selector.query_first(self, root).into_iter().collect()
        }
    }
    pub fn owns(&self, node: &NodeHandle) -> bool {
        Rc::ptr_eq(&self.owner, &node.0.borrow().owner)
    }
    pub fn element_by_id(&self, id: &str) -> Option<NodeHandle> {
        self.root.as_ref()?.element_by_id(id)
    }
    /// Internal identity lookup, not an authored id or a public selector API.
    pub fn lookup(&self, id: Identity) -> Option<NodeHandle> {
        let record = self.records.get(&id)?.upgrade()?;
        if !Rc::ptr_eq(&record.borrow().owner, &self.owner) {
            return None;
        }
        Some(NodeHandle(record))
    }
    pub fn update_slots(&mut self, wire: WireNode, slots: &[Vec<usize>]) -> Result<Node> {
        let next = Prepared::with_slots(wire, &mut slots.iter().cloned())?;
        self.commit(next)
    }
    pub fn update(&mut self, wire: WireNode) -> Result<Node> {
        let next = Prepared::new(wire)?;
        self.commit(next)
    }
    fn commit(&mut self, next: Prepared) -> Result<Node> {
        let root = self.retain(self.root.clone(), next)?;
        self.root = Some(root.clone());
        // Weak index entries neither retain detached nodes nor grow forever.
        self.records.retain(|_, record| record.strong_count() > 0);
        Ok(self
            .lookup(root.0.borrow().id)
            .ok_or("missing retained root")?
            .snapshot())
    }
    fn retain(&mut self, current: Option<NodeHandle>, next: Prepared) -> Result<NodeHandle> {
        let current = current.filter(|node| node.0.borrow().kind == next.kind);
        let node = match current {
            Some(node) => node,
            None => {
                let id = Identity(self.next);
                self.next = self.next.checked_add(1).ok_or("node identity exhausted")?;
                let record = Rc::new(RefCell::new(Record {
                    id,
                    owner: self.owner.clone(),
                    parent: None,
                    kind: next.kind.clone(),
                    slot: next.slot.clone(),
                    authored_classes: next.classes.clone(),
                    authored_attributes: next.attributes.clone(),
                    attributes: next.attributes.clone(),
                    classes: next.classes.clone(),
                    authored_text: next.text.clone(),
                    text: next.text.clone(),
                    style: next.style.clone(),
                    handler: next.handler,
                    children: vec![],
                }));
                self.records.insert(id, Rc::downgrade(&record));
                NodeHandle(record)
            }
        };
        let old_children = node.0.borrow().children.clone();
        let mut children = Vec::with_capacity(next.children.len());
        for (position, child) in next.children.into_iter().enumerate() {
            let old = if child.slot.is_empty() {
                old_children.get(position).cloned()
            } else {
                old_children
                    .iter()
                    .find(|node| node.0.borrow().slot == child.slot)
                    .cloned()
            };
            let kept = self.retain(old, child)?;
            kept.0.borrow_mut().parent = Some(Rc::downgrade(&node.0));
            children.push(kept);
        }
        for old in &old_children {
            if !children.contains(old) {
                old.0.borrow_mut().parent = None;
            }
        }
        let mut record = node.0.borrow_mut();
        record.patch_classes(next.classes, next.style);
        let names: std::collections::BTreeSet<_> = record
            .authored_attributes
            .keys()
            .chain(next.attributes.keys())
            .cloned()
            .collect();
        for name in names {
            if record.authored_attributes.get(&name) != next.attributes.get(&name) {
                match next.attributes.get(&name) {
                    Some(value) => {
                        record.attributes.insert(name.clone(), value.clone());
                    }
                    None => {
                        record.attributes.remove(&name);
                    }
                }
            }
        }
        record.authored_attributes = next.attributes;
        record.patch_text(next.text);
        if record.handler != next.handler {
            record.handler = next.handler;
        }
        if record.children != children {
            record.children = children;
        }
        drop(record);
        Ok(node)
    }
}

impl Record {
    fn patch_text(&mut self, text: Option<String>) -> bool {
        if self.authored_text == text {
            return false;
        }
        self.authored_text = text.clone();
        self.text = text;
        true
    }
    fn patch_classes(&mut self, classes: String, style: Style) -> bool {
        if self.authored_classes == classes {
            return false;
        }
        self.authored_classes = classes.clone();
        self.classes = classes;
        self.style = style;
        true
    }
}
