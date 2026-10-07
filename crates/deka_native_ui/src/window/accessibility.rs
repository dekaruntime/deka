//! AccessKit projection and action ingress for the window event loop.
use crate::{SemanticNode, SemanticRole, scene::Scene};
use accesskit::{Action, Affine, Node, NodeId, Rect, Role, Tree, TreeId, TreeUpdate};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub(super) const ROOT: NodeId = NodeId(0);
#[derive(Default)]
pub(super) struct Projection {
    ids: BTreeMap<String, NodeId>,
    next: u64,
}
impl Projection {
    pub(super) fn id(&mut self, id: &str) -> NodeId {
        if let Some(id) = self.ids.get(id) {
            return *id;
        }
        let node = self.allocate();
        self.ids.insert(id.into(), node);
        node
    }
    pub(super) fn allocate(&mut self) -> NodeId {
        self.next += 1;
        NodeId(self.next)
    }
    pub(super) fn target(&self, id: NodeId) -> Option<&str> {
        self.ids
            .iter()
            .find_map(|(key, value)| (*value == id).then_some(key.as_str()))
    }
    pub(super) fn tree(
        &mut self,
        semantics: &[SemanticNode],
        scene: &Scene,
        focus: Option<&str>,
        scale: f32,
    ) -> TreeUpdate {
        // Entries for deleted nodes are removed; IDs are never reused.
        self.ids
            .retain(|id, _| semantics.iter().any(|n| &n.id == id));
        let mut root = Node::new(Role::Window);
        root.set_label("Deka");
        root.set_bounds(Rect::new(0., 0., scene.width.into(), scene.height.into()));
        root.set_transform(Affine::scale(scale.into()));
        let mut output = TreeUpdate {
            nodes: vec![],
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: focus.map(|id| self.id(id)).unwrap_or(ROOT),
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
            if let Some(bounds) = scene.nodes.iter().find(|n| n.id == item.id) {
                let r = bounds.rect;
                node.set_bounds(Rect::new(
                    r.x.into(),
                    r.y.into(),
                    (r.x + r.width).into(),
                    (r.y + r.height).into(),
                ));
            }
            node.set_children(
                semantics
                    .iter()
                    .filter(|n| !n.hidden && n.parent.as_deref() == Some(&item.id))
                    .map(|n| self.id(&n.id))
                    .collect::<Vec<_>>(),
            );
            if item.parent.is_none() {
                root.push_child(id);
            }
            output.nodes.push((id, node));
        }
        output.nodes.push((ROOT, root));
        output
    }
}
/// Cached full tree makes activation synchronous, without crossing into the
/// main-thread-only retained store from a platform accessibility callback.
pub(super) struct Activation(pub Arc<Mutex<TreeUpdate>>);
impl accesskit::ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        Some(self.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
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
