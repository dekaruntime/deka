//! Retained property bindings for the experimental compiled native program.
use super::{number, render};
use deka_native_ir::{Condition, Node, Number, Program, StyleWhen, Template, Text};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Default, Debug, Serialize)]
pub struct BindingStats {
    pub tree_builds: usize,
    pub nodes_created: usize,
    pub binding_evaluations: usize,
    pub changed_nodes: Vec<String>,
}
enum Value {
    Text(Number),
    Style(Box<StyleWhen>),
}
struct Binding {
    path: Vec<usize>,
    value: Value,
}
pub(super) struct Retained {
    pub root: Node,
    state: Vec<f64>,
    bindings: Vec<Binding>,
    dependents: Vec<Vec<usize>>,
    structural: Vec<bool>,
    pub stats: BindingStats,
}
impl Retained {
    pub fn new(program: &Program, state: &[f64]) -> Self {
        let root = render(&program.root, state);
        let mut tree = Self {
            root,
            state: state.to_vec(),
            bindings: vec![],
            dependents: vec![vec![]; state.len()],
            structural: vec![false; state.len()],
            stats: BindingStats {
                tree_builds: 1,
                ..Default::default()
            },
        };
        fn structure(node: &Template, flags: &mut [bool]) {
            if let Some(Condition::Equal(a, b)) = &node.visible_when {
                for i in dependencies(a).union(&dependencies(b)) {
                    flags[*i] = true;
                }
            }
            for child in &node.children {
                structure(child, flags);
            }
        }
        structure(&program.root, &mut tree.structural);
        tree.collect(&program.root, vec![], state);
        tree.stats.binding_evaluations = tree.bindings.len();
        tree
    }
    fn collect(&mut self, template: &Template, path: Vec<usize>, state: &[f64]) {
        self.stats.nodes_created += 1;
        if let Some(Text::Number(expr)) = &template.text {
            self.bind(path.clone(), Value::Text(expr.clone()), dependencies(expr));
        }
        if let Some(style) = &template.style_when {
            let Condition::Equal(a, b) = &style.condition;
            let deps = dependencies(a).union(&dependencies(b)).copied().collect();
            self.bind(path.clone(), Value::Style(Box::new(style.clone())), deps);
        }
        let visible = template.children.iter().filter(|n| {
            n.visible_when
                .as_ref()
                .is_none_or(|Condition::Equal(a, b)| number(a, state) == number(b, state))
        });
        for (index, child) in visible.enumerate() {
            let mut path = path.clone();
            path.push(index);
            self.collect(child, path, state);
        }
    }
    fn bind(&mut self, path: Vec<usize>, value: Value, deps: BTreeSet<usize>) {
        let index = self.bindings.len();
        for state in deps {
            self.dependents[state].push(index);
        }
        self.bindings.push(Binding { path, value });
    }
    pub fn update(&mut self, program: &Program, state: &[f64]) {
        let changed: Vec<_> = state
            .iter()
            .zip(&self.state)
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect();
        if changed.is_empty() {
            return;
        }
        if changed.iter().any(|i| self.structural[*i]) {
            // Preserve legacy conditional examples. Structural reconciliation is a separate slice;
            // static views use only the property-binding path below.
            let previous = self.stats.clone();
            *self = Self::new(program, state);
            self.stats.tree_builds += previous.tree_builds;
            self.stats.nodes_created += previous.nodes_created;
            self.stats.binding_evaluations += previous.binding_evaluations;
            return;
        }
        self.stats.changed_nodes.clear();
        let affected: BTreeSet<_> = changed
            .iter()
            .flat_map(|i| self.dependents[*i].iter().copied())
            .collect();
        for i in affected {
            let binding = &self.bindings[i];
            let mut node = &mut self.root;
            for index in &binding.path {
                node = &mut node.children[*index];
            }
            match &binding.value {
                Value::Text(expr) => node.text = Some(number(expr, state).to_string()),
                Value::Style(style) => {
                    let Condition::Equal(a, b) = &style.condition;
                    node.style = if number(a, state) == number(b, state) {
                        style.then_style.clone()
                    } else {
                        style.else_style.clone()
                    };
                }
            }
            self.stats.binding_evaluations += 1;
            self.stats.changed_nodes.push(node.id.clone());
        }
        self.state.clone_from_slice(state);
    }
}
fn dependencies(expr: &Number) -> BTreeSet<usize> {
    match expr {
        Number::Literal(_) => BTreeSet::new(),
        Number::State(i) => BTreeSet::from([*i]),
        Number::Add(a, b) | Number::Sub(a, b) | Number::Mul(a, b) => {
            dependencies(a).union(&dependencies(b)).copied().collect()
        }
    }
}
