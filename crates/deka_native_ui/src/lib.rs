//! Native UI shared by interpreted development applications and compiled applications.
pub use deka_native_ir::{Node, Style};
#[cfg(feature = "gpu")]
mod gpu;
#[cfg(feature = "gpu")]
pub use gpu::run;

pub enum Reload {
    Unchanged,
    Preserve,
    Reset,
}
pub trait Application: 'static {
    fn initial_state(&self) -> Vec<f64>;
    fn render(&self, state: &[f64]) -> Node;
    fn event(&self, handler: usize, state: &mut [f64]);
    fn poll_reload(&mut self) -> Reload {
        Reload::Unchanged
    }
    fn live(&self) -> bool {
        false
    }
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
