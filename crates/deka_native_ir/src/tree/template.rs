//! Development edits on the shared store. Plans validate before mutation.
use super::*;

pub enum TemplateEdit {
    Keep(Vec<NodeHandle>),
    Node {
        current: Option<NodeHandle>,
        wire: WireNode,
        slot: Vec<usize>,
        children: Vec<Self>,
    },
}
enum Validated {
    Keep(Vec<NodeHandle>),
    Node {
        current: Option<NodeHandle>,
        next: Box<Prepared>,
        children: Vec<Self>,
    },
}
impl NodeHandle {
    pub fn authored_wire(&self) -> WireNode {
        let record = self.0.borrow();
        WireNode {
            tag: match &record.kind {
                Kind::Element(tag) => tag.clone(),
                Kind::Text => String::new(),
            },
            classes: record.authored_classes.clone(),
            attributes: record.authored_attributes.clone(),
            text: record.authored_text.clone(),
            handler: record.handler,
            children: vec![],
        }
    }
}
impl Tree {
    pub fn patch_template(
        &mut self,
        parent: Option<&NodeHandle>,
        old: &[NodeHandle],
        edits: Vec<TemplateEdit>,
    ) -> Result<Vec<NodeHandle>> {
        fn validate(
            tree: &Tree,
            edit: TemplateEdit,
            used: &mut std::collections::BTreeSet<Identity>,
        ) -> Result<Validated> {
            let mut use_node = |node: &NodeHandle| {
                if !tree.owns(node) {
                    return Err("foreign template node".into());
                }
                if !used.insert(node.0.borrow().id) {
                    return Err("duplicate template node".into());
                }
                Ok(())
            };
            match edit {
                TemplateEdit::Keep(nodes) => {
                    fn subtree(
                        node: &NodeHandle,
                        visit: &mut impl FnMut(&NodeHandle) -> Result<()>,
                    ) -> Result<()> {
                        visit(node)?;
                        for child in node.0.borrow().children.clone() {
                            subtree(&child, visit)?;
                        }
                        Ok(())
                    }
                    for node in &nodes {
                        subtree(node, &mut use_node)?;
                    }
                    Ok(Validated::Keep(nodes))
                }
                TemplateEdit::Node {
                    current,
                    wire,
                    slot,
                    children,
                } => {
                    if let Some(current) = &current {
                        use_node(current)?;
                    }
                    let next = Prepared::with_slots(wire, &mut std::iter::once(slot))?;
                    let children = children
                        .into_iter()
                        .map(|edit| validate(tree, edit, used))
                        .collect::<Result<_>>()?;
                    Ok(Validated::Node {
                        current,
                        next: Box::new(next),
                        children,
                    })
                }
            }
        }
        if parent.is_some_and(|node| !self.owns(node)) || old.iter().any(|node| !self.owns(node)) {
            return Err("foreign template root".into());
        }
        let mut used = std::collections::BTreeSet::new();
        let validated = edits
            .into_iter()
            .map(|edit| validate(self, edit, &mut used))
            .collect::<Result<Vec<_>>>()?;
        let mut ancestor = parent.cloned();
        while let Some(node) = ancestor {
            if used.contains(&node.0.borrow().id) {
                return Err("cyclic template edit".into());
            }
            ancestor = node.parent();
        }
        fn roots(edit: &Validated) -> usize {
            match edit {
                Validated::Keep(nodes) => nodes.len(),
                _ => 1,
            }
        }
        if parent.is_none() && validated.iter().map(roots).sum::<usize>() != 1 {
            return Err("root template requires one root".into());
        }
        fn count(edit: &Validated) -> u64 {
            match edit {
                Validated::Keep(_) => 0,
                Validated::Node { children, .. } => 1 + children.iter().map(count).sum::<u64>(),
            }
        }
        self.next
            .checked_add(validated.iter().map(count).sum())
            .ok_or("node identity exhausted")?;
        fn commit(tree: &mut Tree, edit: Validated) -> Result<Vec<NodeHandle>> {
            match edit {
                Validated::Keep(nodes) => Ok(nodes),
                Validated::Node {
                    current,
                    next,
                    children,
                } => {
                    let saved = current
                        .as_ref()
                        .map(|node| std::mem::take(&mut node.0.borrow_mut().children))
                        .unwrap_or_default();
                    let node = tree.retain(current, *next)?;
                    let mut result = vec![];
                    for edit in children {
                        result.extend(commit(tree, edit)?);
                    }
                    for child in &saved {
                        child.0.borrow_mut().parent = None;
                    }
                    if saved != result {
                        node.reclaim_text_overrides();
                    }
                    node.0.borrow_mut().children = result;
                    Ok(vec![node])
                }
            }
        }
        let mut roots = vec![];
        for edit in validated {
            roots.extend(commit(self, edit)?);
        }
        if let Some(parent) = parent {
            let previous = parent.0.borrow().children.clone();
            let first = previous
                .iter()
                .position(|node| old.contains(node))
                .unwrap_or(previous.len());
            let mut next = vec![];
            for (index, node) in previous.iter().enumerate() {
                if index == first {
                    next.extend(roots.clone());
                }
                if !old.contains(node) {
                    next.push(node.clone());
                }
            }
            if first == previous.len() {
                next.extend(roots.clone());
            }
            if previous != next {
                parent.reclaim_text_overrides();
            }
            parent.0.borrow_mut().children = next;
        } else {
            self.root = Some(roots[0].clone());
        }
        fn parents(
            node: &NodeHandle,
            parent: Option<&NodeHandle>,
            attached: &mut std::collections::BTreeSet<Identity>,
        ) {
            node.0.borrow_mut().parent = parent.map(|p| Rc::downgrade(&p.0));
            attached.insert(node.0.borrow().id);
            for child in node.0.borrow().children.clone() {
                parents(&child, Some(node), attached);
            }
        }
        let mut attached = std::collections::BTreeSet::new();
        if let Some(root) = &self.root {
            parents(root, None, &mut attached);
        }
        for weak in self.records.values() {
            if let Some(record) = weak.upgrade()
                && !attached.contains(&record.borrow().id)
            {
                record.borrow_mut().parent = None;
            }
        }
        self.records.retain(|_, record| record.strong_count() > 0);
        Ok(roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn element(tag: &str) -> WireNode {
        WireNode {
            tag: tag.into(),
            ..Default::default()
        }
    }
    #[test]
    fn a_bad_later_node_leaves_earlier_nodes_and_topology_unchanged() {
        let mut tree = Tree::default();
        tree.update(element("view")).unwrap();
        let root = tree.root.clone().unwrap();
        let before = root.snapshot();
        let mut bad = element("p");
        bad.classes = "p-bad".into();
        let edits = vec![TemplateEdit::Node {
            current: Some(root.clone()),
            wire: WireNode {
                classes: "p-4".into(),
                ..element("view")
            },
            slot: vec![],
            children: vec![TemplateEdit::Node {
                current: None,
                wire: bad,
                slot: vec![0],
                children: vec![],
            }],
        }];
        assert!(
            tree.patch_template(None, std::slice::from_ref(&root), edits)
                .is_err()
        );
        assert_eq!(root.snapshot(), before);
        assert!(tree.root.as_ref() == Some(&root));
    }
    #[test]
    fn opaque_subtree_moves_with_its_identity_and_effective_writes() {
        let mut tree = Tree::default();
        tree.update_slots(
            WireNode {
                children: vec![WireNode {
                    text: Some("state 7".into()),
                    ..Default::default()
                }],
                ..element("view")
            },
            &[vec![], vec![0]],
        )
        .unwrap();
        let root = tree.root.clone().unwrap();
        let state = root.all_children()[0].clone();
        state.set_text_content("effective 9".into());
        let edits = vec![TemplateEdit::Node {
            current: Some(root.clone()),
            wire: root.authored_wire(),
            slot: vec![],
            children: vec![TemplateEdit::Node {
                current: None,
                wire: element("div"),
                slot: vec![1],
                children: vec![TemplateEdit::Keep(vec![state.clone()])],
            }],
        }];
        tree.patch_template(None, &[root], edits).unwrap();
        let wrapper = tree.root.as_ref().unwrap().all_children()[0].clone();
        assert!(wrapper.all_children()[0] == state);
        assert!(state.parent().unwrap() == wrapper);
        assert_eq!(state.text_content(), "effective 9");
    }
}
