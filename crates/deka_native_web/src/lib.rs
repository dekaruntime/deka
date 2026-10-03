//! Browser platform adapter for the same compiler, VM and UI tree as desktop.
use deka_native_ui::scene::{Renderer, Scene};
use deka_vm::{Hosts, compiler, ui::UiSession};
use wasm_bindgen::prelude::*;
mod world;
pub use world::PortfolioWorld;

#[wasm_bindgen]
pub struct NativePreview {
    session: Option<UiSession>,
    renderer: Renderer,
    scene: Scene,
    focus: Option<String>,
}
impl Default for NativePreview {
    fn default() -> Self {
        Self::new()
    }
}
#[wasm_bindgen]
impl NativePreview {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            session: None,
            renderer: Renderer::new(),
            scene: Scene::default(),
            focus: None,
        }
    }
    /// Compile and validate a replacement before touching the running app.
    /// Edits restart VM state. The boolean is retained for host API compatibility.
    pub fn compile(&mut self, source: &str, _reset: bool) -> Result<String, String> {
        if source.len() > 128 * 1024 {
            return Err("Preview source limit is 128 KiB".into());
        }
        let program = compiler::compile_entry(source, &Hosts::default(), "App")?;
        let replacement = UiSession::new(program)?;
        let status = if self.session.is_some() {
            "reset"
        } else {
            "started"
        };
        self.session = Some(replacement);
        self.scene = Scene::default();
        self.focus = None;
        self.renderer.reset_animations();
        Ok(status.into())
    }
    pub fn frame(&mut self, width: f32, height: f32, scale: f32) -> String {
        self.frame_at(width, height, scale, 0., true)
    }
    /// Explicit presentation clock for browser animation and deterministic CI.
    pub fn frame_at(
        &mut self,
        width: f32,
        height: f32,
        scale: f32,
        milliseconds: f64,
        reduced_motion: bool,
    ) -> String {
        if let Some(session) = &self.session {
            self.scene = self.renderer.render_at(
                session.tree(),
                width,
                height,
                scale,
                milliseconds,
                reduced_motion,
            );
            if let Some(id) = &self.focus {
                self.scene.focus_ring(id);
            }
        }
        serde_json::to_string(&self.scene).expect("finite scene")
    }
    pub fn binding_stats(&self) -> String {
        let stats = self.session.as_ref().map(|s| {
            serde_json::json!({
                "instructions": s.instructions(), "binding_evaluations": s.evaluations(),
                "heap": s.stats(), "runtime": "deka_vm"
            })
        });
        serde_json::to_string(&stats).expect("VM diagnostics")
    }
    pub fn pointer(&mut self, x: f32, y: f32) -> Result<bool, String> {
        if let Some(target) = self.scene.hit(x, y) {
            self.focus = Some(target.id.clone());
            if let Some(session) = &mut self.session {
                session.click(target.handler)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Return false at tab boundaries to let focus leave the canvas.
    pub fn key(&mut self, key: &str, backwards: bool) -> Result<bool, String> {
        if key == "Tab" {
            let current = self
                .scene
                .targets
                .iter()
                .position(|t| Some(&t.id) == self.focus.as_ref());
            let next = match (current, backwards) {
                (None, false) => Some(0),
                (None, true) => self.scene.targets.len().checked_sub(1),
                (Some(i), false) => Some(i + 1),
                (Some(i), true) => i.checked_sub(1),
            };
            self.focus = next
                .and_then(|i| self.scene.targets.get(i))
                .map(|t| t.id.clone());
            return Ok(self.focus.is_some());
        }
        if matches!(key, "Enter" | " ")
            && let Some(target) = self
                .scene
                .targets
                .iter()
                .find(|t| Some(&t.id) == self.focus.as_ref())
            && let Some(session) = &mut self.session
        {
            session.click(target.handler)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub fn blur(&mut self) {
        self.focus = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deka_vm::ui;
    const SOURCE: &str = r#"export fn App() {
        let count = 0;
        return (<view className="p-4 gap-2"><p>{count}</p>
            <button onClick={fn() { count += 1; }}>Add one</button>
            {count == 1 ? <p className="enter-fade duration-500">Visible</p> : None}
        </view>);
    }"#;
    fn snapshot(preview: &mut NativePreview) -> serde_json::Value {
        serde_json::from_str(&preview.frame(560., 300., 1.)).unwrap()
    }
    fn normalize(mut scene: serde_json::Value) -> serde_json::Value {
        scene["images"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|i| i["id"].as_str().unwrap().to_owned());
        scene
    }
    #[test]
    fn desktop_and_browser_execute_identical_source_events_and_scenes() {
        for count in [0, 1, 3] {
            let program = compiler::compile_entry(SOURCE, &Hosts::default(), "App").unwrap();
            let native: serde_json::Value = serde_json::from_str(
                &ui::snapshot(ui::VmApp::new(program).unwrap(), count).unwrap(),
            )
            .unwrap();
            let mut browser = NativePreview::new();
            browser.compile(SOURCE, false).unwrap();
            for _ in 0..count {
                snapshot(&mut browser);
                let r = browser.scene.targets[0].rect;
                assert!(browser.pointer(r.x + 2., r.y + 2.).unwrap());
            }
            browser.blur();
            assert_eq!(normalize(snapshot(&mut browser)), normalize(native));
            let instructions = browser.session.as_ref().unwrap().instructions();
            for time in 0..60 {
                browser.frame_at(560., 300., 1., time as f64 * 16., false);
            }
            assert_eq!(
                browser.session.as_ref().unwrap().instructions(),
                instructions
            );
        }
    }
    #[test]
    fn browser_events_patch_only_bindings_affected_by_their_state() {
        let source = r#"export fn App(){let count=0;let noise=0;
            return (<view><p>{count}</p>
                <button onClick={fn(){noise+=1;}}>Noise</button>
                <button onClick={fn(){count+=1;}}>Count</button></view>);
        }"#;
        let mut browser = NativePreview::new();
        browser.compile(source, false).unwrap();
        snapshot(&mut browser);
        let before = browser.session.as_ref().unwrap().evaluations();
        let noise = browser.scene.targets[0].rect;
        browser.pointer(noise.x + 2., noise.y + 2.).unwrap();
        snapshot(&mut browser);
        assert_eq!(browser.session.as_ref().unwrap().evaluations(), before);
        let count = browser.scene.targets[1].rect;
        browser.pointer(count.x + 2., count.y + 2.).unwrap();
        snapshot(&mut browser);
        assert_eq!(browser.session.as_ref().unwrap().evaluations(), before + 1);
        assert!(
            browser
                .scene
                .nodes
                .iter()
                .any(|node| node.text.as_deref() == Some("1"))
        );
        let instructions = browser.session.as_ref().unwrap().instructions();
        for clock in 0..60 {
            browser.frame_at(560., 300., 1., clock as f64 * 16., false);
        }
        assert_eq!(
            browser.session.as_ref().unwrap().instructions(),
            instructions
        );
    }
    #[test]
    fn failed_edits_keep_working_state_valid_edits_reset_and_keyboard_works() {
        let mut browser = NativePreview::new();
        browser.compile(SOURCE, false).unwrap();
        snapshot(&mut browser);
        assert!(browser.key("Tab", false).unwrap());
        assert!(browser.key("Enter", false).unwrap());
        let good = snapshot(&mut browser);
        for source in [
            "export fn App() { !!!",
            &SOURCE.replace("p-4", "unsupported-class"),
        ] {
            assert!(browser.compile(source, false).is_err());
            assert_eq!(normalize(snapshot(&mut browser)), normalize(good.clone()));
        }
        assert_eq!(browser.compile(SOURCE, false).unwrap(), "reset");
        assert!(
            snapshot(&mut browser)["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["text"] == "0")
        );
        assert!(!browser.pointer(900., 900.).unwrap());
    }
    #[test]
    fn browser_frames_preserve_retained_identity_and_new_nodes_do_not_reuse_it() {
        let source = r#"export fn App() {
            let visible = true;
            return (<view><button onClick={fn() {visible = visible == false;}}>Toggle</button>
                {visible ? <p>Hello</p> : None}</view>);
        }"#;
        let mut browser = NativePreview::new();
        browser.compile(source, false).unwrap();
        let initial = snapshot(&mut browser);
        let button_id = browser.scene.targets[0].id.clone();
        let old_text_id = browser
            .scene
            .nodes
            .iter()
            .find(|node| node.text.as_deref() == Some("Hello"))
            .unwrap()
            .id
            .clone();
        for time in 0..30 {
            assert_eq!(
                normalize(snapshot(&mut browser)),
                normalize(initial.clone())
            );
            browser.frame_at(560., 300., 1., time as f64 * 16., true);
        }
        for _ in 0..2 {
            let target = browser.scene.targets[0].rect;
            assert!(browser.pointer(target.x + 2., target.y + 2.).unwrap());
            snapshot(&mut browser);
            assert_eq!(browser.scene.targets[0].id, button_id);
        }
        let shown = browser
            .scene
            .nodes
            .iter()
            .find(|node| node.text.as_deref() == Some("Hello"))
            .unwrap();
        assert_ne!(shown.id, old_text_id);
    }
}
