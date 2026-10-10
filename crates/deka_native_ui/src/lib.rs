//! Native UI shared by interpreted development applications and compiled applications.
pub use deka_native_ir::{
    Align, CLASS_ATTRIBUTE, Edges, Justify, Keyframe, Length, Motion, Node, Style,
};
pub mod animation;
pub mod geometry;
mod layout;
mod layout_motion;
mod motion;
mod text;
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
pub mod window;
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
pub use window::{Snapshot, run, snapshot};

/// Wakes whatever drives an [`Application`] (the desktop window's event loop)
/// from any thread, so it runs the application's background turn. The window
/// hands one to [`Application::set_waker`]; the VM registers it as its wake.
#[derive(Clone)]
pub struct Waker(std::sync::Arc<dyn Fn() + Send + Sync>);
impl Waker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(wake))
    }
    pub fn wake(&self) {
        (self.0)()
    }
}

pub enum Reload {
    Unchanged,
    Preserve,
    Reset,
}
/// Text editing metadata from the application's retained tree.
#[derive(Clone, Debug)]
pub struct TextControl {
    pub controlled: bool,
    pub id: String,
    pub value: String,
    pub placeholder: String,
    /// Effective text foreground, including inherited colour.
    pub color: u32,
    pub multiline: bool,
}
/// Backend-independent semantics from the effective retained tree.
#[derive(Clone, Debug)]
pub struct SemanticNode {
    pub id: String,
    pub parent: Option<String>,
    pub role: SemanticRole,
    pub name: String,
    pub value: String,
    pub disabled: bool,
    pub hidden: bool,
    pub tab_index: Option<i32>,
    pub clickable: bool,
    pub selected: Option<bool>,
    pub controls: Option<String>,
    pub expanded: Option<bool>,
    pub has_popup: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticRole {
    Group,
    Label,
    Button,
    TextInput,
    MultilineTextInput,
    List,
    ListItem,
    TabList,
    Tab,
    TabPanel,
    Status,
    Dialog,
    Menu,
    MenuItem,
}
/// Positive tab indices precede natural source order; negative indices allow
/// explicit focus only. Disabled/hidden nodes never receive focus/actions.
pub fn tab_order(nodes: &[SemanticNode]) -> Vec<String> {
    let mut focusable: Vec<_> = nodes
        .iter()
        .filter(|n| !n.disabled && !n.hidden && n.tab_index.is_some_and(|i| i >= 0))
        .collect();
    focusable.sort_by_key(|n| match n.tab_index {
        Some(i) if i > 0 => (0, i),
        _ => (1, 0),
    });
    focusable.into_iter().map(|n| n.id.clone()).collect()
}
/// Host-owned modal focus history, shared by native and browser sessions.
#[derive(Default)]
pub struct ModalFocus {
    stack: Vec<(String, Option<String>)>,
}
impl ModalFocus {
    pub fn synchronize(&mut self, nodes: &[SemanticNode], focused: Option<&str>) -> Option<String> {
        let modal = modal_root(nodes);
        let previous = self.stack.last().map(|(id, _)| id.as_str());
        let mut restore = None;
        if previous != modal {
            if let Some(index) = self
                .stack
                .iter()
                .position(|(id, _)| Some(id.as_str()) == modal)
            {
                while self.stack.len() > index + 1 {
                    restore = self.stack.pop().and_then(|(_, saved)| saved);
                }
            } else if let Some(id) = modal {
                self.stack.push((id.into(), focused.map(str::to_owned)));
            } else {
                while let Some((_, saved)) = self.stack.pop() {
                    restore = saved;
                }
            }
            if let Some(id) = restore.filter(|id| focus_available(nodes, id)) {
                return Some(id);
            }
        }
        if modal.is_some() && focused.is_none_or(|id| !focus_available(nodes, id)) {
            return tab_order(nodes)
                .into_iter()
                .next()
                .or_else(|| modal.map(str::to_owned));
        }
        None
    }
}
pub fn modal_root(nodes: &[SemanticNode]) -> Option<&str> {
    nodes
        .iter()
        .rev()
        .find(|n| n.role == SemanticRole::Dialog && !n.hidden)
        .map(|n| n.id.as_str())
}
pub fn focus_available(nodes: &[SemanticNode], id: &str) -> bool {
    nodes
        .iter()
        .any(|n| n.id == id && n.tab_index.is_some() && !n.hidden && !n.disabled)
}
/// Modal boundaries wrap; ordinary boundaries let focus leave the application.
pub fn next_tab(
    targets: &[String],
    current: Option<&str>,
    backwards: bool,
    wrap: bool,
) -> Option<String> {
    let position = targets.iter().position(|id| Some(id.as_str()) == current);
    let next = match (position, backwards) {
        (None, false) => Some(0),
        (None, true) => targets.len().checked_sub(1),
        (Some(i), false) => Some(i + 1),
        (Some(i), true) => i.checked_sub(1),
    };
    next.and_then(|i| targets.get(i))
        .or_else(|| {
            wrap.then(|| {
                if backwards {
                    targets.last()
                } else {
                    targets.first()
                }
            })
            .flatten()
        })
        .cloned()
}
pub trait Application: 'static {
    fn initial_state(&self) -> Vec<f64>;
    fn render(&self, state: &[f64]) -> Node;
    fn event(&self, handler: usize, state: &mut [f64]);
    fn semantics(&self) -> Vec<SemanticNode> {
        vec![]
    }
    /// Operational host failures use the application's ordinary error sink.
    fn report_error(&self, operation: &str, message: String) {
        eprintln!("deka {operation}: {message}");
    }
    fn text_controls(&self) -> Vec<TextControl> {
        vec![]
    }
    fn text_input(&self, _id: &str, _value: String) -> bool {
        false
    }
    fn context_menu(&self, _id: &str, _x: f32, _y: f32) -> bool {
        false
    }
    fn key_input(&self, _id: &str, _key: String) -> bool {
        false
    }
    /// Consume an explicit focus request on the UI thread. Backends validate
    /// current semantics before focusing, including hidden/disabled ancestry.
    fn take_focus_request(&self) -> Option<String> {
        None
    }
    /// Backend-independent application scheduling. Backends supply a wake and
    /// drive finite turns; applications keep their task ownership internally.
    fn has_ready_work(&self) -> bool {
        false
    }
    fn run_turn(&mut self, _budget: usize) -> bool {
        false
    }
    fn poll_reload(&mut self) -> Reload {
        Reload::Unchanged
    }
    fn live(&self) -> bool {
        false
    }
    /// Called once by the window with a [`Waker`] for asynchronous work.
    fn set_waker(&mut self, _waker: Waker) {}
}
pub struct Host<A: Application> {
    pub app: A,
    pub state: Vec<f64>,
}
impl<A: Application> Host<A> {
    pub fn new(app: A) -> Self {
        let state = app.initial_state();
        Self { app, state }
    }
    pub fn render(&self) -> Node {
        self.app.render(&self.state)
    }
    pub fn click(&mut self, handler: usize) {
        self.app.event(handler, &mut self.state);
    }
    pub fn set_waker(&mut self, waker: Waker) {
        self.app.set_waker(waker);
    }
    pub fn has_ready_work(&self) -> bool {
        self.app.has_ready_work()
    }
    pub fn run_turn(&mut self, budget: usize) -> bool {
        self.app.run_turn(budget)
    }
    pub fn refresh(&mut self) -> bool {
        match self.app.poll_reload() {
            Reload::Unchanged => false,
            Reload::Preserve => true,
            Reload::Reset => {
                self.state = self.app.initial_state();
                true
            }
        }
    }
}
/// Render and invoke the application's actual handlers without requiring a display server.
/// Used by the experimental host's explicit `--exercise` verification mode.
pub fn exercise<A: Application>(app: A, clicks: usize) -> String {
    let mut host = Host::new(app);
    for _ in 0..clicks {
        if let Some(handler) = first_handler(&host.render()) {
            host.click(handler);
        }
    }
    let mut text = String::new();
    collect_text(&host.render(), &mut text);
    text
}
fn first_handler(node: &Node) -> Option<usize> {
    node.on_click
        .or_else(|| node.children.iter().find_map(first_handler))
}
fn collect_text(node: &Node, output: &mut String) {
    if let Some(text) = &node.text {
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(text);
    }
    for child in &node.children {
        collect_text(child, output);
    }
}

#[cfg(feature = "program")]
pub mod program;
pub mod scene;

/// Isolated portfolio-world proof of concept.
pub mod world;
