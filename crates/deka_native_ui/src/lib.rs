//! Native UI shared by interpreted development applications and compiled applications.
pub use deka_native_ir::{Align, Edges, Justify, Keyframe, Length, Motion, Node, Style};
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
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticRole {
    Group,
    Label,
    Button,
    TextInput,
    MultilineTextInput,
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
pub trait Application: 'static {
    fn initial_state(&self) -> Vec<f64>;
    fn render(&self, state: &[f64]) -> Node;
    fn event(&self, handler: usize, state: &mut [f64]);
    fn semantics(&self) -> Vec<SemanticNode> {
        vec![]
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
