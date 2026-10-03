//! Rust-owned node identities live here; renderer Nodes are disposable snapshots.
//!
//! Structural slots retain static siblings when an earlier conditional is absent.
//! Dynamic list positions are not author keys; reordered lists are still positional.
use crate::{Result, ui::WireNode};
use deka_native_ui::{Node, Style};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity(u64);

#[derive(PartialEq, Eq)]
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
    authored_text: Option<String>,
    authored_classes: String,
    kind: Kind,
    slot: Vec<usize>,
    classes: String,
    style: Style,
    text: Option<String>,
    handler: Option<usize>,
    children: Vec<Identity>,
}

#[derive(Default)]
pub(crate) struct Tree {
    root: Option<Identity>,
    records: BTreeMap<Identity, Record>,
    next: u64,
}
impl Tree {
    pub(crate) fn update_slots(&mut self, wire: WireNode, slots: &[Vec<usize>]) -> Result<Node> {
        let next = Prepared::with_slots(wire, &mut slots.iter().cloned())?;
        self.commit(next)
    }
    #[cfg(any(test, feature = "v8-control"))]
    pub(crate) fn update(&mut self, wire: WireNode) -> Result<Node> {
        // Validate the whole authored frame before changing the lasting tree.
        let next = Prepared::new(wire)?;
        self.commit(next)
    }
    fn commit(&mut self, next: Prepared) -> Result<Node> {
        let root = self.retain(self.root, next)?;
        if let Some(old) = self.root.filter(|old| *old != root) {
            self.remove(old);
        }
        self.root = Some(root);
        Ok(self.snapshot(root))
    }
    fn retain(&mut self, current: Option<Identity>, next: Prepared) -> Result<Identity> {
        let current = current.filter(|id| {
            self.records
                .get(id)
                .is_some_and(|record| record.kind == next.kind)
        });
        let id = match current {
            Some(id) => id,
            None => {
                let id = Identity(self.next);
                self.next = self.next.checked_add(1).ok_or("node identity exhausted")?;
                id
            }
        };
        let old_children = current
            .and_then(|id| self.records.get(&id))
            .map(|record| record.children.clone())
            .unwrap_or_default();
        let mut children = Vec::with_capacity(next.children.len());
        for (position, child) in next.children.into_iter().enumerate() {
            let old = if child.slot.is_empty() {
                old_children.get(position).copied()
            } else {
                old_children
                    .iter()
                    .copied()
                    .find(|id| self.records.get(id).is_some_and(|r| r.slot == child.slot))
            };
            let kept = self.retain(old, child)?;
            if let Some(old) = old.filter(|old| *old != kept) {
                self.remove(old);
            }
            children.push(kept);
        }
        for old in &old_children {
            if !children.contains(old) {
                self.remove(*old);
            }
        }
        if let Some(record) = self.records.get_mut(&id) {
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
        } else {
            self.records.insert(
                id,
                Record {
                    kind: next.kind,
                    slot: next.slot,
                    authored_classes: next.classes.clone(),
                    classes: next.classes,
                    authored_text: next.text.clone(),
                    style: next.style,
                    text: next.text,
                    handler: next.handler,
                    children,
                },
            );
        }
        Ok(id)
    }
    fn remove(&mut self, id: Identity) {
        if let Some(record) = self.records.remove(&id) {
            for child in record.children {
                self.remove(child);
            }
        }
    }
    fn snapshot(&self, id: Identity) -> Node {
        let record = &self.records[&id];
        Node {
            id: format!("view/{}", id.0),
            style: record.style.clone(),
            text: record.text.clone(),
            on_click: record.handler,
            children: record
                .children
                .iter()
                .map(|child| self.snapshot(*child))
                .collect(),
        }
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
        let root = tree.root.unwrap();
        let mut invalid = element("view", vec![element("unknown", vec![])]);
        invalid.classes = "p-4".into();
        assert!(tree.update(invalid).is_err());
        assert_eq!(tree.snapshot(root), first);
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
        let first = app.render().unwrap().root;
        let paragraph = app
            .tree
            .records
            .iter()
            .find(|(id, _)| format!("view/{}", id.0) == first.children[0].id)
            .map(|(id, _)| *id)
            .unwrap();
        let leaf = app.tree.records[&paragraph].children[0];
        let hand_style = WireNode {
            tag: "p".into(),
            classes: "text-[#00ff00]".into(),
            ..Default::default()
        }
        .style()
        .unwrap();
        app.tree.records.get_mut(&paragraph).unwrap().style = hand_style.clone();
        app.tree.records.get_mut(&paragraph).unwrap().classes = "text-[#00ff00]".into();
        app.tree.records.get_mut(&leaf).unwrap().text = Some("Hand edit".into());
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
