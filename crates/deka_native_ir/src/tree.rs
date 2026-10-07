//! Rust-owned node identities live here; renderer Nodes are disposable snapshots.
//!
//! Structural slots retain static siblings when an earlier conditional is absent.
//! Dynamic list positions are not author keys; reordered lists are still positional.
use crate::{Node, Style, WireNode};
type Result<T> = std::result::Result<T, String>;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Identity(u64);

#[derive(Clone, PartialEq, Eq)]
enum Kind {
    Text,
    Element(String),
}

/// Effective element text replaces visible children while retaining their
/// authored identities for a later binding change to reclaim the property.
struct TextOverride(String);

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
    text_override: Option<TextOverride>,
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
#[cfg(test)]
thread_local! {
    static SLOT_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static HANDLE_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl PartialEq for NodeHandle {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(test)]
        HANDLE_COMPARISONS.with(|count| count.set(count.get() + 1));
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for NodeHandle {}
impl NodeHandle {
    pub fn slot(&self) -> Vec<usize> {
        #[cfg(test)]
        SLOT_CLONES.with(|count| count.set(count.get() + 1));
        self.0.borrow().slot.clone()
    }
    pub fn all_children(&self) -> Vec<Self> {
        let record = self.0.borrow();
        if record.text_override.is_some() {
            vec![]
        } else {
            record.children.clone()
        }
    }
    /// Effective writes do not change the last authored binding values.
    pub fn set_text_content(&self, text: String) -> bool {
        let mut record = self.0.borrow_mut();
        if matches!(record.kind, Kind::Text) {
            if record.text.as_ref() == Some(&text) {
                return false;
            }
            record.text = Some(text);
        } else {
            if record
                .text_override
                .as_ref()
                .is_some_and(|value| value.0 == text)
            {
                return false;
            }
            record.text_override = Some(TextOverride(text));
        }
        true
    }
    pub fn set_attribute(&self, name: &str, value: String) -> Result<bool> {
        let mut record = self.0.borrow_mut();
        if !matches!(record.kind, Kind::Element(_)) {
            return Err("attributes require an element".into());
        }
        if !matches!(name, "id" | "className" | "value" | "placeholder") {
            return Err(format!("unsupported native attribute {name}"));
        }
        if name == "className" {
            let Kind::Element(tag) = &record.kind else {
                return Err("classes require an element".into());
            };
            let style = WireNode {
                tag: tag.clone(),
                classes: value.clone(),
                ..Default::default()
            }
            .style()?;
            if record.attributes.get(name) == Some(&value) {
                return Ok(false);
            }
            record.classes = value.clone();
            record.style = style;
        } else if record.attributes.get(name) == Some(&value) {
            return Ok(false);
        }
        record.attributes.insert(name.into(), value);
        Ok(true)
    }
    fn reclaim_text_overrides(&self) {
        self.0.borrow_mut().text_override = None;
    }
    fn reclaim_text_binding(&self) {
        self.reclaim_text_overrides();
        // A direct text binding owns its text node and containing element's
        // text presentation. It does not own any more distant ancestor.
        if matches!(self.0.borrow().kind, Kind::Text)
            && let Some(parent) = self.0.borrow().parent.as_ref().and_then(Weak::upgrade)
        {
            Self(parent).reclaim_text_overrides();
        }
    }
    /// Patch one authored text property, preserving effective edits until its
    /// authored value changes. This is the same ownership rule used by retain.
    pub fn patch_text(&self, text: String) -> bool {
        let changed = self.0.borrow_mut().patch_text(Some(text));
        if changed {
            self.reclaim_text_binding();
        }
        changed
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
        let parent = self.0.borrow().parent.as_ref()?.upgrade().map(Self)?;
        if parent.0.borrow().text_override.is_some() {
            None
        } else {
            Some(parent)
        }
    }
    pub fn children(&self) -> Vec<Self> {
        self.all_children()
            .into_iter()
            .filter(Self::is_element)
            .collect()
    }
    pub fn text_content(&self) -> String {
        let record = self.0.borrow();
        if let Some(text) = &record.text_override {
            return text.0.clone();
        }
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
        self.all_children()
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
            text: record
                .text_override
                .as_ref()
                .map(|value| value.0.clone())
                .or_else(|| record.text.clone()),
            on_click: record.handler,
            children: if record.text_override.is_some() {
                vec![]
            } else {
                record.children.iter().map(Self::snapshot).collect()
            },
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
        node.all_children()
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
        let old = parent.0.borrow().children.clone();
        let mut by_slot = HashMap::with_capacity(old.len());
        for node in &old {
            // Clone a path once per child, rather than for every candidate match.
            by_slot.entry(node.slot()).or_insert_with(|| node.clone());
        }
        let mut replacements = Vec::with_capacity(prepared.len());
        for next in prepared {
            let current = by_slot.remove(&next.slot);
            let node = self.retain(current, next)?;
            node.0.borrow_mut().parent = Some(Rc::downgrade(&parent.0));
            replacements.push(node);
        }
        let kept: HashSet<_> = replacements.iter().map(|node| node.0.as_ptr()).collect();
        let mut untouched = Vec::with_capacity(old.len());
        for node in old {
            if node.0.borrow().slot.starts_with(prefix) {
                if !kept.contains(&node.0.as_ptr()) {
                    node.0.borrow_mut().parent = None;
                }
            } else {
                untouched.push(node);
            }
        }
        // Authored structural paths arrive in order. Merge the unchanged and
        // replaced sequences linearly; retain the old sorting semantics for
        // unusual callers supplying unordered paths, without cloning paths.
        let compare = |a: &NodeHandle, b: &NodeHandle| a.0.borrow().slot.cmp(&b.0.borrow().slot);
        if !untouched.is_sorted_by(|a, b| compare(a, b).is_le()) {
            untouched.sort_by(compare);
        }
        let mut ordered = replacements.clone();
        if !ordered.is_sorted_by(|a, b| compare(a, b).is_le()) {
            ordered.sort_by(compare);
        }
        let mut children = Vec::with_capacity(untouched.len() + ordered.len());
        let mut untouched = untouched.into_iter().peekable();
        let mut ordered = ordered.into_iter().peekable();
        while let (Some(a), Some(b)) = (untouched.peek(), ordered.peek()) {
            if compare(a, b).is_le() {
                children.extend(untouched.next());
            } else {
                children.extend(ordered.next());
            }
        }
        children.extend(untouched);
        children.extend(ordered);
        let structural_change = parent.0.borrow().children != children;
        parent.0.borrow_mut().children = children;
        if structural_change {
            parent.reclaim_text_overrides();
        }
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
                    text_override: None,
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
        let mut by_slot = HashMap::with_capacity(old_children.len());
        for old in &old_children {
            by_slot.entry(old.slot()).or_insert_with(|| old.clone());
        }
        let mut children = Vec::with_capacity(next.children.len());
        for (position, child) in next.children.into_iter().enumerate() {
            let old = if child.slot.is_empty() {
                old_children.get(position).cloned()
            } else {
                by_slot.remove(&child.slot)
            };
            let kept = self.retain(old, child)?;
            kept.0.borrow_mut().parent = Some(Rc::downgrade(&node.0));
            children.push(kept);
        }
        let kept: HashSet<_> = children.iter().map(|child| child.0.as_ptr()).collect();
        for old in &old_children {
            if !kept.contains(&old.0.as_ptr()) {
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
        let text_changed = record.patch_text(next.text);
        if record.handler != next.handler {
            record.handler = next.handler;
        }
        let children_changed = record.children != children;
        if children_changed {
            record.children = children;
        }
        drop(record);
        if text_changed {
            node.reclaim_text_binding();
        }
        if children_changed {
            node.reclaim_text_overrides();
        }
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

#[cfg(test)]
mod replacement_tests {
    use super::*;
    fn text(value: impl ToString) -> WireNode {
        WireNode {
            text: Some(value.to_string()),
            ..Default::default()
        }
    }
    #[test]
    fn large_slot_replacement_has_linear_path_clones_and_identity_checks() {
        for size in [64, 1024] {
            let mut tree = Tree::default();
            let mut children = vec![text("Before")];
            children.extend((0..size).map(text));
            children.push(text("After"));
            let mut slots = vec![vec![], vec![0]];
            slots.extend((0..size).map(|i| vec![1, i]));
            slots.push(vec![2]);
            tree.update_slots(
                WireNode {
                    tag: "view".into(),
                    children,
                    ..Default::default()
                },
                &slots,
            )
            .unwrap();
            let parent = tree.root.clone().unwrap();
            let before = parent.all_children();
            SLOT_CLONES.with(|count| count.set(0));
            HANDLE_COMPARISONS.with(|count| count.set(0));
            let next = tree
                .replace_slot(
                    &parent,
                    &[1],
                    (0..size - 1).map(|i| text(i + 1)).collect(),
                    &(0..size - 1).map(|i| vec![1, i]).collect::<Vec<_>>(),
                )
                .unwrap();
            assert!(
                SLOT_CLONES.with(|count| count.get()) <= 4 * size,
                "quadratic slot cloning"
            );
            assert!(
                HANDLE_COMPARISONS.with(|count| count.get()) <= 4 * size,
                "quadratic identity checks"
            );
            let after = parent.all_children();
            assert!(after.first() == before.first());
            assert!(after.last() == before.last());
            for (i, node) in next.iter().enumerate() {
                assert!(node == &before[i + 1]);
                assert_eq!(node.text_content(), (i + 1).to_string());
                assert!(node.parent() == Some(parent.clone()));
            }
            assert!(before[size].parent().is_none());
            assert_eq!(
                parent.text_content(),
                "Before".to_owned()
                    + &(1..size).map(|i| i.to_string()).collect::<String>()
                    + "After"
            );
        }
    }
    #[test]
    fn unsorted_slots_keep_ordering_compatibility_without_quadratic_clones() {
        let mut tree = Tree::default();
        tree.update_slots(
            WireNode {
                tag: "view".into(),
                children: vec![text("After"), text("Before")],
                ..Default::default()
            },
            &[vec![], vec![2], vec![0]],
        )
        .unwrap();
        let parent = tree.root.clone().unwrap();
        tree.replace_slot(
            &parent,
            &[1],
            vec![text("B"), text("A")],
            &[vec![1, 1], vec![1, 0]],
        )
        .unwrap();
        assert_eq!(parent.text_content(), "BeforeABAfter");
    }
}
