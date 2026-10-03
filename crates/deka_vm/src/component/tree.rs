//! Rust-owned node identities live here; renderer Nodes are disposable snapshots.
//!
//! Keep the current positional matching policy. This does not promise keyed
//! identity for reordered lists or expose an author-facing node-handle API.
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
    style: Style,
    text: Option<String>,
    handler: Option<usize>,
    children: Vec<Prepared>,
}
impl Prepared {
    fn new(wire: WireNode) -> Result<Self> {
        let kind = if wire.text.is_some() {
            Kind::Text
        } else {
            Kind::Element(wire.tag.clone())
        };
        let style = wire.style()?;
        Ok(Self {
            kind,
            style,
            text: wire.text,
            handler: wire.handler,
            children: wire
                .children
                .into_iter()
                .map(Self::new)
                .collect::<Result<_>>()?,
        })
    }
}
struct Record {
    kind: Kind,
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
    pub(crate) fn update(&mut self, wire: WireNode) -> Result<Node> {
        // Validate the whole authored frame before changing the lasting tree.
        let next = Prepared::new(wire)?;
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
            let old = old_children.get(position).copied();
            let kept = self.retain(old, child)?;
            if let Some(old) = old.filter(|old| *old != kept) {
                self.remove(old);
            }
            children.push(kept);
        }
        for old in &old_children[children.len().min(old_children.len())..] {
            self.remove(*old);
        }
        if let Some(record) = self.records.get_mut(&id) {
            record.style = next.style;
            record.text = next.text;
            record.handler = next.handler;
            record.children = children;
        } else {
            self.records.insert(
                id,
                Record {
                    kind: next.kind,
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
