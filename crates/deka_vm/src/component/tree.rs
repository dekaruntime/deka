//! Rust-owned node identities live here; renderer Nodes are disposable snapshots.
//!
//! Structural slots retain static siblings when an earlier conditional is absent.
//! Dynamic list positions are not author keys; reordered lists are still positional.
use crate::{Result, ui::WireNode};
use deka_native_ui::{Node, Style};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity(u64);

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
    content: String,
}
impl Prepared {
    #[cfg(any(test, feature = "v8-control"))]
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
        let content = wire.text.clone().unwrap_or_default()
            + &wire
                .children
                .iter()
                .filter_map(|child| child.text.as_deref())
                .collect::<String>();
        Ok(Self {
            kind,
            content,
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
#[derive(Clone)]
pub(super) struct Record {
    id: Identity,
    owner: Rc<Owner>,
    parent: Option<Weak<RefCell<Record>>>,
    authored_text: Option<String>,
    authored_classes: String,
    authored_attributes: BTreeMap<String, String>,
    attributes: BTreeMap<String, String>,
    kind: Kind,
    slot: Vec<usize>,
    classes: String,
    style: Style,
    text: Option<String>,
    handler: Option<usize>,
    children: Vec<NodeHandle>,
    authored_children: Vec<Weak<RefCell<Record>>>,
    authored_content: String,
    text_override: bool,
    callback: Option<crate::heap::Handle>,
    callback_scope: Weak<()>,
    input: Option<(crate::heap::Handle, crate::heap::Handle)>,
}

/// Owner tokens distinguish separate sessions even when both number their root 0.
#[derive(Default)]
struct Owner;
/// One allocation is one node. Children own their subtree; parent/index edges
/// are weak. The renderer receives snapshots, never these resource references.
#[derive(Clone)]
pub(crate) struct NodeHandle(pub(super) Rc<RefCell<Record>>);
impl PartialEq for NodeHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for NodeHandle {}
impl NodeHandle {
    pub(super) fn is_element(&self) -> bool {
        matches!(self.0.borrow().kind, Kind::Element(_))
    }
    pub(super) fn parent(&self) -> Option<Self> {
        self.0.borrow().parent.as_ref()?.upgrade().map(Self)
    }
    pub(super) fn children(&self) -> Vec<Self> {
        self.0
            .borrow()
            .children
            .iter()
            .filter(|node| node.is_element())
            .cloned()
            .collect()
    }
    pub(super) fn text_content(&self) -> String {
        let record = self.0.borrow();
        record.text.clone().unwrap_or_default()
            + &record
                .children
                .iter()
                .map(Self::text_content)
                .collect::<String>()
    }
    pub(super) fn attribute(&self, name: &str) -> Option<String> {
        let record = self.0.borrow();
        if name == "className" {
            record.attributes.get(name).map(|_| record.classes.clone())
        } else {
            record.attributes.get(name).cloned()
        }
    }
    pub(super) fn element_by_id(&self, id: &str) -> Option<Self> {
        if self.is_element() && self.attribute("id").as_deref() == Some(id) {
            return Some(self.clone());
        }
        self.0
            .borrow()
            .children
            .iter()
            .find_map(|node| node.element_by_id(id))
    }
    pub(super) fn scope(&self) -> Weak<()> {
        self.0.borrow().callback_scope.clone()
    }
    pub(super) fn roots(&self, scope: &Rc<()>) -> Vec<crate::heap::Handle> {
        let record = self.0.borrow();
        let mut roots = vec![];
        if record
            .callback_scope
            .upgrade()
            .is_some_and(|owner| Rc::ptr_eq(&owner, scope))
        {
            roots.extend(record.callback);
            if let Some((change, key)) = record.input {
                roots.extend([change, key]);
            }
        }
        for child in &record.children {
            roots.extend(child.roots(scope));
        }
        roots
    }
    fn snapshot(&self) -> Node {
        Self::snapshot_record(&self.0.borrow())
    }
    fn snapshot_record(record: &Record) -> Node {
        Node {
            id: format!("view/{}", record.id.0),
            style: record.style.clone(),
            text: record.text.clone(),
            on_click: if record.callback.is_some()
                || matches!(&record.kind, Kind::Element(tag) if tag == "input")
            {
                Some(event_id(record.id, 0))
            } else {
                record.handler
            },
            children: record.children.iter().map(Self::snapshot).collect(),
        }
    }
}
#[derive(Default)]
pub(crate) struct Tree {
    root: Option<NodeHandle>,
    records: BTreeMap<Identity, Weak<RefCell<Record>>>,
    next: u64,
    owner: Rc<Owner>,
    authored_nodes: Vec<NodeHandle>,
    callbacks: Vec<crate::heap::Handle>,
    scope: Weak<()>,
    inputs: Vec<(usize, crate::heap::Handle, crate::heap::Handle)>,
}
impl Tree {
    pub(super) fn owns(&self, node: &NodeHandle) -> bool {
        Rc::ptr_eq(&self.owner, &node.0.borrow().owner)
    }
    pub(super) fn element_by_id(&self, id: &str) -> Option<NodeHandle> {
        self.root.as_ref()?.element_by_id(id)
    }
    pub(crate) fn set_scope(&mut self, scope: &Rc<()>) {
        self.scope = Rc::downgrade(scope);
    }
    pub(crate) fn set_callbacks(
        &mut self,
        callbacks: &[crate::heap::Handle],
        inputs: &[super::InputSpec],
    ) {
        self.callbacks = callbacks.to_vec();
        self.inputs = inputs
            .iter()
            .map(|input| (input.target, callbacks[input.change], callbacks[input.key]))
            .collect();
    }
    pub(crate) fn roots(&self) -> Vec<crate::heap::Handle> {
        let mut todo: Vec<_> = self
            .root
            .iter()
            .chain(self.authored_nodes.iter())
            .cloned()
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut roots = vec![];
        while let Some(node) = todo.pop() {
            let record = node.0.borrow();
            if !seen.insert(record.id) {
                continue;
            }
            roots.extend(record.callback);
            if let Some((change, key)) = record.input {
                roots.extend([change, key]);
            }
            todo.extend(record.children.iter().cloned());
        }
        roots
    }
    pub(crate) fn is_input_event(&self, event: usize) -> bool {
        event & EVENT_BIT != 0
            && self
                .lookup(Identity(((event & !EVENT_BIT) >> 2) as u64))
                .is_some_and(
                    |node| matches!(&node.0.borrow().kind, Kind::Element(tag) if tag == "input"),
                )
    }
    pub(crate) fn event(&self, event: usize) -> Option<crate::heap::Handle> {
        if event & EVENT_BIT == 0 {
            return None;
        }
        let part = event & 3;
        let node = self.lookup(Identity(((event & !EVENT_BIT) >> 2) as u64))?;
        let record = node.0.borrow();
        match part {
            0 => record.callback,
            1 => record.input.map(|input| input.0),
            2 => record.input.map(|input| input.1),
            _ => None,
        }
    }
    pub(crate) fn input_specs(&self) -> Vec<super::InputSpec> {
        fn walk(node: &NodeHandle, inputs: &mut Vec<super::InputSpec>) {
            let record = node.0.borrow();
            if matches!(&record.kind, Kind::Element(tag) if tag == "input") {
                inputs.push(super::InputSpec {
                    target: event_id(record.id, 0),
                    value: record.attributes.get("value").cloned().unwrap_or_default(),
                    placeholder: record
                        .attributes
                        .get("placeholder")
                        .cloned()
                        .unwrap_or_default(),
                    change: event_id(record.id, 1),
                    key: event_id(record.id, 2),
                });
            }
            for child in &record.children {
                walk(child, inputs);
            }
        }
        let mut inputs = vec![];
        if let Some(root) = &self.root {
            walk(root, &mut inputs);
        }
        inputs
    }
    pub(super) fn create(&mut self, tag: &str) -> Result<NodeHandle> {
        let next = Prepared::with_slots(
            WireNode {
                tag: tag.into(),
                ..Default::default()
            },
            &mut std::iter::repeat(vec![]),
        )?;
        self.retain(None, next, &mut vec![])
    }
    pub(super) fn set_text(&mut self, node: &NodeHandle, text: String) -> Result<()> {
        if node.is_element() {
            let child = if text.is_empty() {
                None
            } else {
                let wire = WireNode {
                    text: Some(text),
                    ..Default::default()
                };
                Some(self.retain(
                    None,
                    Prepared::with_slots(wire, &mut std::iter::repeat(vec![]))?,
                    &mut vec![],
                )?)
            };
            let effective = node.0.borrow().children.clone();
            for child in effective {
                detach(&child);
            }
            if let Some(child) = child {
                attach(node, &child, None)?;
            }
            node.0.borrow_mut().text_override = true;
        } else {
            node.0.borrow_mut().text = Some(text);
        }
        Ok(())
    }
    pub(super) fn set_attribute(
        &mut self,
        node: &NodeHandle,
        name: &str,
        value: String,
    ) -> Result<()> {
        if !crate::ui_contract::SCALAR_ATTRIBUTES.contains(&name) {
            return Err(format!("unsupported view attribute: {name}"));
        }
        let mut record = node.0.borrow_mut();
        if name == "className" {
            let Kind::Element(tag) = &record.kind else {
                return Err("expected view element".into());
            };
            let style = WireNode {
                tag: tag.clone(),
                classes: value.clone(),
                ..Default::default()
            }
            .style()?;
            record.classes = value.clone();
            record.style = style;
        }
        record.attributes.insert(name.into(), value);
        Ok(())
    }
    pub(super) fn edit_class(&mut self, node: &NodeHandle, token: &str, add: bool) -> Result<()> {
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err("class token must be nonempty without whitespace".into());
        }
        let mut tokens: Vec<_> = node
            .0
            .borrow()
            .classes
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        if !add && tokens.is_empty() && node.attribute("className").is_none() {
            return Ok(());
        }
        if add {
            if !tokens.iter().any(|old| old == token) {
                tokens.push(token.into());
            }
        } else {
            tokens.retain(|old| old != token);
        }
        self.set_attribute(node, "className", tokens.join(" "))
    }
    pub(super) fn insert(
        &mut self,
        parent: &NodeHandle,
        child: &NodeHandle,
        reference: Option<&NodeHandle>,
    ) -> Result<NodeHandle> {
        if !parent.is_element() {
            return Err("parent must be a view element".into());
        }
        if let Some(reference) = reference
            && reference.parent().as_ref() != Some(parent)
        {
            return Err("reference is not a child of this parent".into());
        }
        let mut cursor = Some(parent.clone());
        while let Some(node) = cursor {
            if node == *child {
                return Err("view insertion would create a cycle".into());
            }
            cursor = node.parent();
        }
        if self.root.as_ref() == Some(child) {
            return Err("cannot reparent the mounted view root".into());
        }
        if reference != Some(child) {
            attach(parent, child, reference)?;
        }
        Ok(child.clone())
    }
    pub(super) fn remove(&mut self, parent: &NodeHandle, child: &NodeHandle) -> Result<NodeHandle> {
        if child.parent().as_ref() != Some(parent) {
            return Err("node is not a child of this parent".into());
        }
        detach(child);
        Ok(child.clone())
    }
    /// Internal identity lookup, not an authored id or a public selector API.
    fn lookup(&self, id: Identity) -> Option<NodeHandle> {
        let record = self.records.get(&id)?.upgrade()?;
        if !Rc::ptr_eq(&record.borrow().owner, &self.owner) {
            return None;
        }
        Some(NodeHandle(record))
    }
    pub(crate) fn update_slots(&mut self, wire: WireNode, slots: &[Vec<usize>]) -> Result<Node> {
        let next = Prepared::with_slots(wire, &mut slots.iter().cloned())?;
        self.commit(next)
    }
    #[cfg(any(test, feature = "v8-control"))]
    pub(crate) fn update(&mut self, wire: WireNode) -> Result<Node> {
        let next = Prepared::new(wire)?;
        self.commit(next)
    }
    fn commit(&mut self, next: Prepared) -> Result<Node> {
        let mut authored_nodes = vec![];
        // Preparation validates scalar/style values before mutation. Retaining
        // can still reject a structural cycle caused by an imperative move.
        // Keep the complete effective store intact if a binding cannot commit.
        let saved: Vec<_> = self
            .records
            .values()
            .filter_map(Weak::upgrade)
            .map(|node| {
                let record = node.borrow().clone();
                (node, record)
            })
            .collect();
        let root = match self.retain(self.root.clone(), next, &mut authored_nodes) {
            Ok(root) => root,
            Err(error) => {
                for (node, record) in saved {
                    *node.borrow_mut() = record;
                }
                return Err(error);
            }
        };
        drop(saved);
        self.authored_nodes = authored_nodes;
        self.root = Some(root.clone());
        // Weak index entries neither retain detached nodes nor grow forever.
        self.records.retain(|_, record| record.strong_count() > 0);
        Ok(self
            .lookup(root.0.borrow().id)
            .ok_or("missing retained root")?
            .snapshot())
    }
    fn retain(
        &mut self,
        current: Option<NodeHandle>,
        next: Prepared,
        authored_nodes: &mut Vec<NodeHandle>,
    ) -> Result<NodeHandle> {
        let current = current.filter(|node| node.0.borrow().kind == next.kind);
        let node = match current {
            Some(node) => node,
            None => {
                if self.next >= (EVENT_BIT >> 2) as u64 {
                    return Err("node identity exhausted".into());
                }
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
                    authored_children: vec![],
                    authored_content: next.content.clone(),
                    text_override: false,
                    callback: None,
                    callback_scope: self.scope.clone(),
                    input: None,
                }));
                self.records.insert(id, Rc::downgrade(&record));
                NodeHandle(record)
            }
        };
        authored_nodes.push(node.clone());
        let old_children: Vec<_> = node
            .0
            .borrow()
            .authored_children
            .iter()
            .filter_map(|weak| weak.upgrade().map(NodeHandle))
            .collect();
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
            let kept = self.retain(old, child, authored_nodes)?;
            children.push(kept);
        }
        let restore_text = {
            let record = node.0.borrow();
            record.text_override
                && (record.authored_content != next.content || old_children != children)
        };
        if restore_text {
            let effective = node.0.borrow().children.clone();
            for child in effective {
                detach(&child);
            }
            for child in &children {
                attach(&node, child, None)?;
            }
        } else if old_children != children && !node.0.borrow().text_override {
            for old in &old_children {
                if !children.contains(old) && old.parent().as_ref() == Some(&node) {
                    detach(old);
                }
            }
            for (index, child) in children.iter().enumerate() {
                if !old_children.contains(child) {
                    let reference = children[index + 1..]
                        .iter()
                        .find(|next| next.parent().as_ref() == Some(&node))
                        .cloned();
                    attach(&node, child, reference.as_ref())?;
                }
            }
        }
        let mut record = node.0.borrow_mut();
        if record.authored_classes != next.classes
            || record.authored_attributes.get("className") != next.attributes.get("className")
        {
            record.authored_classes = next.classes.clone();
            record.classes = next.classes;
            record.style = next.style;
        }
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
        if record.authored_text != next.text {
            record.authored_text = next.text.clone();
            record.text = next.text;
        }
        record.handler = next.handler;
        record.callback = next
            .handler
            .and_then(|index| self.callbacks.get(index).copied());
        record.input = next.handler.and_then(|target| {
            self.inputs
                .iter()
                .find(|(id, _, _)| *id == target)
                .map(|(_, change, key)| (*change, *key))
        });
        record.authored_children = children
            .iter()
            .map(|child| Rc::downgrade(&child.0))
            .collect();
        record.authored_content = next.content;
        if restore_text {
            record.text_override = false;
        }
        drop(record);
        Ok(node)
    }
}

const EVENT_BIT: usize = 1 << (usize::BITS - 2);
fn event_id(id: Identity, part: usize) -> usize {
    EVENT_BIT | ((id.0 as usize) << 2) | part
}
fn detach(child: &NodeHandle) {
    if let Some(parent) = child.parent() {
        parent.0.borrow_mut().children.retain(|node| node != child);
    }
    child.0.borrow_mut().parent = None;
}
fn attach(parent: &NodeHandle, child: &NodeHandle, reference: Option<&NodeHandle>) -> Result<()> {
    let mut cursor = Some(parent.clone());
    while let Some(node) = cursor {
        if node == *child {
            return Err("view insertion would create a cycle".into());
        }
        cursor = node.parent();
    }
    detach(child);
    let mut record = parent.0.borrow_mut();
    let position = reference
        .and_then(|reference| record.children.iter().position(|node| node == reference))
        .unwrap_or(record.children.len());
    record.children.insert(position, child.clone());
    child.0.borrow_mut().parent = Some(Rc::downgrade(&parent.0));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn element(tag: &str, children: Vec<WireNode>) -> WireNode {
        WireNode {
            tag: tag.into(),
            children,
            ..Default::default()
        }
    }
    #[test]
    fn a_binding_cycle_after_a_hand_move_rolls_back_the_complete_frame() {
        fn text(value: &str) -> WireNode {
            WireNode {
                text: Some(value.into()),
                ..Default::default()
            }
        }
        let wire = |value: &str| {
            element(
                "view",
                vec![
                    element("p", vec![text(value)]),
                    element("div", vec![element("div", vec![text(value)]), text(value)]),
                ],
            )
        };
        let mut tree = Tree::default();
        tree.update(wire("First")).unwrap();
        let root = tree.root.clone().unwrap();
        let a = root.0.borrow().children[1].clone();
        let b = a.0.borrow().children[0].clone();
        tree.insert(&root, &b, None).unwrap();
        tree.insert(&b, &a, None).unwrap();
        tree.set_text(&a, "Manual".into()).unwrap();
        let before = root.snapshot();
        assert!(tree.update(wire("Next")).unwrap_err().contains("cycle"));
        assert_eq!(root.snapshot(), before);
        assert!(a.parent().unwrap() == b);
        assert!(b.parent().unwrap() == root);
    }
    #[test]
    fn removed_subtrees_are_released_and_a_failed_frame_leaves_the_store_unchanged() {
        let mut tree = Tree::default();
        let first = tree
            .update(element(
                "view",
                vec![element("div", vec![element("p", vec![])])],
            ))
            .unwrap();
        assert_eq!(tree.records.len(), 3);
        let root = tree.root.clone().unwrap();
        let mut invalid = element("view", vec![element("unknown", vec![])]);
        invalid.classes = "p-4".into();
        assert!(tree.update(invalid).is_err());
        assert_eq!(root.snapshot(), first);
        drop(root);
        assert_eq!(tree.records.len(), 3);
        let empty = tree.update(element("view", vec![])).unwrap();
        assert_eq!(empty.id, first.id);
        assert_eq!(tree.records.len(), 1);
        let next = tree.update(element("div", vec![])).unwrap();
        assert_ne!(next.id, first.id);
        assert_eq!(tree.records.len(), 1);
    }
}

#[cfg(all(test, feature = "compiler"))]
mod ownership_tests {
    use super::*;
    use crate::{Hosts, compiler, component::Component};
    #[test]
    fn imperative_values_survive_until_the_last_authored_value_changes() {
        let mut app=Component::new(compiler::compile_entry(r#"export fn App() {
            let count=0; let red=false; let noise=0;
            return {view:fn(){return (<view><p className={red ? "text-[#ff0000]" : "text-[#0000ff]"}>{count % 2}</p></view>);},
                count:fn(){count+=1;}, red:fn(){red=red==false;}, noise:fn(){noise+=1;count+=2;} };
        }"#,&Hosts::default(),"App").unwrap(),Hosts::default()).unwrap();
        app.render().unwrap();
        let paragraph = app.tree.borrow().root.as_ref().unwrap().0.borrow().children[0].clone();
        let leaf = paragraph.0.borrow().children[0].clone();
        let hand_style = WireNode {
            tag: "p".into(),
            classes: "text-[#00ff00]".into(),
            ..Default::default()
        }
        .style()
        .unwrap();
        paragraph.0.borrow_mut().style = hand_style.clone();
        paragraph.0.borrow_mut().classes = "text-[#00ff00]".into();
        leaf.0.borrow_mut().text = Some("Hand edit".into());
        // No VM work is needed for the next snapshot to expose direct Rust edits.
        let before = app.instructions();
        let edited = app.render().unwrap().root;
        assert_eq!(app.instructions(), before);
        assert_eq!(edited.children[0].style, hand_style);
        assert_eq!(
            edited.children[0].children[0].text.as_deref(),
            Some("Hand edit")
        );
        app.call("noise", vec![]).unwrap();
        let unchanged = app.render().unwrap().root;
        assert_eq!(unchanged.children[0].style, hand_style);
        assert_eq!(
            unchanged.children[0].children[0].text.as_deref(),
            Some("Hand edit")
        );
        app.call("count", vec![]).unwrap();
        let changed = app.render().unwrap().root;
        assert_eq!(changed.children[0].children[0].text.as_deref(), Some("1"));
        assert_eq!(changed.children[0].style, hand_style);
        app.call("red", vec![]).unwrap();
        let changed = app.render().unwrap().root;
        assert_ne!(changed.children[0].style, hand_style);
        assert_eq!(changed.children[0].children[0].text.as_deref(), Some("1"));
    }
}

#[cfg(test)]
mod handle_tests {
    use super::*;
    fn source() -> WireNode {
        WireNode {
            tag: "view".into(),
            children: vec![WireNode {
                tag: "p".into(),
                children: vec![WireNode {
                    text: Some("Hello".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn handles_retain_one_node_and_its_subtree_without_retaining_its_parent() {
        let mut tree = Tree::default();
        tree.update(source()).unwrap();
        let root = tree.root.clone().unwrap();
        let node = root.0.borrow().children[0].clone();
        let alias = tree.lookup(node.0.borrow().id).unwrap();
        assert!(node == alias);
        let weak = Rc::downgrade(&node.0);
        let child = Rc::downgrade(&node.0.borrow().children[0].0);
        let parent = Rc::downgrade(&root.0);
        tree.update(WireNode {
            tag: "view".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(node.0.borrow().parent.is_none());
        assert_eq!(node.snapshot().children[0].text.as_deref(), Some("Hello"));
        assert!(tree.lookup(node.0.borrow().id).unwrap() == node);
        tree.update(source()).unwrap();
        assert!(tree.root.as_ref().unwrap().0.borrow().children[0] != node);
        drop(alias);
        drop(root);
        drop(tree);
        assert!(parent.upgrade().is_none());
        assert!(weak.upgrade().is_some());
        assert!(child.upgrade().is_some());
        drop(node);
        assert!(weak.upgrade().is_none());
        assert!(child.upgrade().is_none());
    }
    #[test]
    fn sessions_never_alias_and_weak_indices_release_removed_nodes() {
        let mut first = Tree::default();
        let mut second = Tree::default();
        first.update(source()).unwrap();
        second.update(source()).unwrap();
        let a = first.root.clone().unwrap();
        let b = second.root.clone().unwrap();
        assert_eq!(a.snapshot().id, b.snapshot().id);
        assert!(a != b);
        let old = Rc::downgrade(&a.0.borrow().children[0].0);
        for _ in 0..100 {
            first
                .update(WireNode {
                    tag: "view".into(),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(first.records.len(), 1);
            first.update(source()).unwrap();
            assert_eq!(first.records.len(), 3);
        }
        assert!(old.upgrade().is_none());
        drop(first);
        assert!(
            a.0.borrow().children[0]
                .0
                .borrow()
                .parent
                .as_ref()
                .unwrap()
                .upgrade()
                .is_some()
        );
        let child = a.0.borrow().children[0].clone();
        let oldroot = Rc::downgrade(&a.0);
        drop(a);
        assert!(oldroot.upgrade().is_none());
        assert!(
            child
                .0
                .borrow()
                .parent
                .as_ref()
                .unwrap()
                .upgrade()
                .is_none()
        );
        assert_eq!(child.snapshot().children[0].text.as_deref(), Some("Hello"));
    }
}

#[cfg(all(test, feature = "compiler"))]
mod typed_node_tests {
    use super::*;
    use crate::{
        HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, compiler, component::Component,
    };
    const SOURCE: &str = r#"import {nodeText} from "vm:host";
        export fn App(){
            let visible=true; let title="Hello"; let held: Option<TestNode> = None;
            return {view:fn(){return (<view>{visible ? <p>{title}</p> : None}</view>);},
                retain:fn(node:TestNode){held=Some(node);},
                read:fn(){return match(held){Some(node)=>node.snapshotText(),None=>"Empty"};},
                rename:fn(){title="Changed";}, forget:fn(){held=None;}, hide:fn(){visible=false;} };
        }"#;
    fn catalog() -> Hosts {
        let mut hosts = Hosts::default();
        hosts
            .register(
                HostOp::new(
                    "nodeText",
                    vec![HostType::Handle("TestNode".into())],
                    HostType::String,
                    false,
                    |args| {
                        let HostValue::Handle(handle) = &args[0] else {
                            return HostReply::Ready(Err("expected node handle".into()));
                        };
                        let Some(record) = handle.downcast_ref::<RefCell<Record>>() else {
                            return HostReply::Ready(Err("invalid node resource".into()));
                        };
                        let snapshot = NodeHandle::snapshot_record(&record.borrow());
                        fn text(node: &Node) -> String {
                            node.text.clone().unwrap_or_default()
                                + &node.children.iter().map(text).collect::<String>()
                        }
                        HostReply::Ready(Ok(HostValue::String(text(&snapshot))))
                    },
                )
                .with_receiver_method("TestNode", "snapshotText"),
            )
            .unwrap();
        hosts
            .register(HostOp::new(
                "otherNode",
                vec![],
                HostType::Handle("OtherNode".into()),
                false,
                |_| HostReply::Ready(Ok(HostValue::Handle(HostHandle::new("OtherNode", 0u8)))),
            ))
            .unwrap();
        hosts
    }
    #[test]
    fn a_typed_vm_alias_retains_a_real_detached_node_until_collection() {
        let hosts = catalog();
        let mut app = Component::new(
            compiler::compile_entry(SOURCE, &hosts, "App").unwrap(),
            hosts,
        )
        .unwrap();
        app.render().unwrap();
        let node = app.tree.borrow().root.as_ref().unwrap().0.borrow().children[0].clone();
        let id = node.0.borrow().id;
        let weak = Rc::downgrade(&node.0);
        let a = HostHandle::from_shared("TestNode", node.0.clone());
        let b = HostHandle::from_shared("TestNode", app.tree.borrow().lookup(id).unwrap().0);
        assert_eq!(a, b); // Repeated lookups name one allocation, not proxy wrappers.
        app.call("retain", vec![HostValue::Handle(a)]).unwrap();
        drop(b);
        drop(node);
        app.call("rename", vec![]).unwrap();
        app.render().unwrap();
        let current = app.tree.borrow().root.as_ref().unwrap().0.borrow().children[0].clone();
        assert!(Rc::ptr_eq(&weak.upgrade().unwrap(), &current.0));
        drop(current);
        assert_eq!(
            app.call("read", vec![]).unwrap(),
            HostValue::String("Changed".into())
        );
        app.call("hide", vec![]).unwrap();
        app.render().unwrap();
        assert!(weak.upgrade().is_some());
        assert_eq!(
            app.call("read", vec![]).unwrap(),
            HostValue::String("Changed".into())
        );
        app.call("forget", vec![]).unwrap();
        app.render().unwrap();
        assert!(weak.upgrade().is_none());
        assert!(app.tree.borrow().lookup(id).is_none());
    }
    #[test]
    fn node_values_cannot_be_forged_as_records_or_replaced_with_other_brands() {
        let hosts = catalog();
        for value in ["\"fake\"", "{value:1}", "1", "otherNode()"] {
            let source = SOURCE
                .replace("{nodeText}", "{nodeText, otherNode}")
                .replace("held=Some(node);", &format!("held=Some({value});"));
            let error = compiler::compile_entry(&source, &hosts, "App").unwrap_err();
            assert!(error.contains("TestNode"), "{error}");
        }
    }
}
