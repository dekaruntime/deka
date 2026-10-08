//! AccessKit projection and action ingress for the window event loop.
use crate::{SemanticNode, SemanticRole, scene::Scene};
use accesskit::{Action, Affine, Node, NodeId, Rect, Role, Tree, TreeId, TreeUpdate};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const ROOT: NodeId = NodeId(0);
#[derive(Default)]
pub(super) struct Projection {
    ids: BTreeMap<String, NodeId>,
    next: u64,
    targets: BTreeMap<NodeId, String>,
    sent: BTreeMap<NodeId, Node>,
    initialized: bool,
}
impl Projection {
    pub(super) fn deactivate(&mut self) {
        self.ids.clear();
        self.targets.clear();
        self.sent.clear();
        self.initialized = false;
        // Keep the allocator monotonic so queued requests cannot target a new
        // control after an accessibility activation session has ended.
    }
    pub(super) fn id(&mut self, id: &str) -> NodeId {
        if let Some(id) = self.ids.get(id) {
            return *id;
        }
        let node = self.allocate();
        self.ids.insert(id.into(), node);
        self.targets.insert(node, id.into());
        node
    }
    pub(super) fn allocate(&mut self) -> NodeId {
        self.next += 1;
        NodeId(self.next)
    }
    pub(super) fn target(&self, id: NodeId) -> Option<&str> {
        self.targets.get(&id).map(String::as_str)
    }
    /// Compare after editor text runs/selection have joined the update.
    pub(super) fn incremental(&mut self, mut update: TreeUpdate) -> TreeUpdate {
        let current: BTreeMap<_, _> = update.nodes.iter().cloned().collect();
        if self.initialized {
            update.tree = None;
            update
                .nodes
                .retain(|(id, node)| self.sent.get(id) != Some(node));
        }
        self.sent = current;
        self.initialized = true;
        update
    }

    pub(super) fn tree(
        &mut self,
        semantics: &[SemanticNode],
        scene: &Scene,
        focus: Option<&str>,
        scale: f32,
    ) -> TreeUpdate {
        // Map each relationship once. Deleted identities lose their reverse route.
        let present: BTreeSet<_> = semantics.iter().map(|n| n.id.as_str()).collect();
        self.ids.retain(|id, node| {
            if present.contains(id.as_str()) {
                true
            } else {
                self.targets.remove(node);
                false
            }
        });
        let visible: BTreeSet<_> = semantics
            .iter()
            .filter(|n| !n.hidden)
            .map(|n| n.id.as_str())
            .collect();
        let mut children: BTreeMap<Option<&str>, Vec<NodeId>> = BTreeMap::new();
        for item in semantics.iter().filter(|n| !n.hidden) {
            let parent = item.parent.as_deref().filter(|id| visible.contains(id));
            children.entry(parent).or_default().push(self.id(&item.id));
        }
        let bounds: BTreeMap<_, _> = scene
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.rect))
            .collect();
        let mut root = Node::new(Role::Window);
        root.set_label("Deka");
        root.set_bounds(Rect::new(0., 0., scene.width.into(), scene.height.into()));
        root.set_transform(Affine::scale(scale.into()));
        let mut output = TreeUpdate {
            nodes: vec![],
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: focus
                .filter(|id| visible.contains(id))
                .and_then(|id| self.ids.get(id).copied())
                .unwrap_or(ROOT),
        };
        for item in semantics.iter().filter(|n| !n.hidden) {
            let id = self.id(&item.id);
            let mut node = Node::new(match item.role {
                SemanticRole::Group => Role::GenericContainer,
                SemanticRole::Label => Role::Label,
                SemanticRole::Button => Role::Button,
                SemanticRole::TextInput => Role::TextInput,
                SemanticRole::MultilineTextInput => Role::MultilineTextInput,
            });
            if !item.name.is_empty() {
                node.set_label(item.name.clone());
            }
            if matches!(
                item.role,
                SemanticRole::TextInput | SemanticRole::MultilineTextInput
            ) {
                node.set_value(item.value.clone());
            }
            if item.disabled {
                node.set_disabled();
            } else {
                if item.tab_index.is_some() {
                    node.add_action(Action::Focus);
                }
                if item.clickable {
                    node.add_action(Action::Click);
                }
                if matches!(
                    item.role,
                    SemanticRole::TextInput | SemanticRole::MultilineTextInput
                ) {
                    node.add_action(Action::SetValue);
                    node.add_action(Action::SetTextSelection);
                }
            }
            if let Some(r) = bounds.get(item.id.as_str()) {
                node.set_bounds(Rect::new(
                    r.x.into(),
                    r.y.into(),
                    (r.x + r.width).into(),
                    (r.y + r.height).into(),
                ));
            }
            node.set_children(children.remove(&Some(item.id.as_str())).unwrap_or_default());
            output.nodes.push((id, node));
        }
        root.set_children(children.remove(&None).unwrap_or_default());
        output.nodes.push((ROOT, root));
        output
    }
}
pub(super) fn empty_tree() -> TreeUpdate {
    TreeUpdate {
        nodes: vec![(ROOT, Node::new(Role::Window))],
        tree: Some(Tree::new(ROOT)),
        tree_id: TreeId::ROOT,
        focus: ROOT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removing_a_node_drops_its_reverse_route_and_parent_child_edge() {
        let mut projection = Projection::default();
        let item = SemanticNode {
            id: "removed".into(),
            parent: None,
            role: SemanticRole::Button,
            name: "Temporary".into(),
            value: String::new(),
            disabled: false,
            hidden: false,
            tab_index: Some(0),
            clickable: true,
        };
        let first = projection.tree(&[item], &Scene::default(), Some("removed"), 1.);
        let removed = first.focus;
        assert_eq!(projection.target(removed), Some("removed"));
        projection.incremental(first);
        let next = projection.tree(&[], &Scene::default(), Some("removed"), 1.);
        let next = projection.incremental(next);
        assert_eq!(projection.target(removed), None);
        assert_eq!(next.focus, ROOT);
        assert!(next.tree.is_none());
        assert!(
            next.nodes
                .iter()
                .find(|(id, _)| *id == ROOT)
                .unwrap()
                .1
                .children()
                .is_empty()
        );
    }
}
