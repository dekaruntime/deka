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
    style: Style,
    text: Option<String>,
    handler: Option<usize>,
    children: Vec<Prepared>,
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
        Ok(Self {
            kind,
            slot,
            classes: wire.classes,
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
struct Record {
    id: Identity,
    owner: Rc<Owner>,
    parent: Option<Weak<RefCell<Record>>>,
    authored_text: Option<String>,
    authored_classes: String,
    kind: Kind,
    slot: Vec<usize>,
    classes: String,
    style: Style,
    text: Option<String>,
    handler: Option<usize>,
    children: Vec<NodeHandle>,
}

/// Owner tokens distinguish separate sessions even when both number their root 0.
#[derive(Default)]
struct Owner;
/// One allocation is one node. Children own their subtree; parent/index edges
/// are weak. The renderer receives snapshots, never these resource references.
#[derive(Clone)]
pub(crate) struct NodeHandle(Rc<RefCell<Record>>);
impl PartialEq for NodeHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for NodeHandle {}
impl NodeHandle {
    fn snapshot(&self) -> Node {
        Self::snapshot_record(&self.0.borrow())
    }
    fn snapshot_record(record: &Record) -> Node {
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
pub(crate) struct Tree {
    root: Option<NodeHandle>,
    records: BTreeMap<Identity, Weak<RefCell<Record>>>,
    next: u64,
    owner: Rc<Owner>,
}
impl Tree {
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
        if record.authored_classes != next.classes {
            record.authored_classes = next.classes.clone();
            record.classes = next.classes;
            record.style = next.style;
        }
        if record.authored_text != next.text {
            record.authored_text = next.text.clone();
            record.text = next.text;
        }
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
        let paragraph = app.tree.root.as_ref().unwrap().0.borrow().children[0].clone();
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
                    None,
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
                None,
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
        let node = app.tree.root.as_ref().unwrap().0.borrow().children[0].clone();
        let id = node.0.borrow().id;
        let weak = Rc::downgrade(&node.0);
        let a = HostHandle::from_shared("TestNode", node.0.clone());
        let b = HostHandle::from_shared("TestNode", app.tree.lookup(id).unwrap().0);
        assert_eq!(a, b); // Repeated lookups name one allocation, not proxy wrappers.
        app.call("retain", vec![HostValue::Handle(a)]).unwrap();
        drop(b);
        drop(node);
        app.call("rename", vec![]).unwrap();
        app.render().unwrap();
        let current = app.tree.root.as_ref().unwrap().0.borrow().children[0].clone();
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
        assert!(app.tree.lookup(id).is_none());
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
