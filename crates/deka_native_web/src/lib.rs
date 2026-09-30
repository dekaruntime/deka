//! Browser host: source compiler + portable native runtime. No DOM component mapping.
use deka_native_ui::{
    Application, Host, Reload,
    program::ProgramApp,
    scene::{Renderer, Scene},
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct NativePreview {
    host: Option<Host<ProgramApp>>,
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
            host: None,
            renderer: Renderer::new(),
            scene: Scene::default(),
            focus: None,
        }
    }
    /// A compiler error leaves the live application and its state untouched.
    pub fn compile(&mut self, source: &str, reset: bool) -> Result<String, String> {
        if source.len() > 128 * 1024 {
            return Err("Native preview source limit is 128 KiB".into());
        }
        let program = deka_native_compile::compile(source)?;
        let status = if let Some(host) = &mut self.host {
            let reload = host.app.replace(program)?;
            if reset || matches!(reload, Reload::Reset) {
                host.state = host.app.initial_state();
                self.focus = None;
                "reset"
            } else {
                "preserved"
            }
        } else {
            self.host = Some(Host::new(ProgramApp::new(program)?));
            "started"
        };
        Ok(status.into())
    }
    pub fn frame(&mut self, width: f32, height: f32, scale: f32) -> String {
        if let Some(host) = &self.host {
            self.scene = self.renderer.render(&host.render(), width, height, scale);
            if let Some(id) = &self.focus {
                self.scene.focus_ring(id);
            }
        }
        serde_json::to_string(&self.scene).expect("finite scene")
    }
    pub fn pointer(&mut self, x: f32, y: f32) -> bool {
        if let Some(target) = self.scene.hit(x, y) {
            self.focus = Some(target.id.clone());
            if let Some(host) = &mut self.host {
                host.click(target.handler);
                return true;
            }
        }
        false
    }
    /// Return false at a tab boundary so the host can let focus leave the canvas.
    pub fn key(&mut self, key: &str, backwards: bool) -> bool {
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
            return self.focus.is_some();
        }
        if matches!(key, "Enter" | " ")
            && let Some(target) = self
                .scene
                .targets
                .iter()
                .find(|t| Some(&t.id) == self.focus.as_ref())
            && let Some(host) = &mut self.host
        {
            host.click(target.handler);
            return true;
        }

        false
    }
    pub fn blur(&mut self) {
        self.focus = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = include_str!("../../../examples/native/counter.dsx");
    fn frame(preview: &mut NativePreview) -> serde_json::Value {
        serde_json::from_str(&preview.frame(560., 300., 2.)).unwrap()
    }
    #[test]
    fn pixels_events_reload_and_errors_share_the_native_host() {
        let mut preview = NativePreview::new();
        preview.compile(SOURCE, false).unwrap();
        let before = frame(&mut preview);
        assert!(!before["paint"].as_array().unwrap().is_empty());
        assert!(before["images"].as_array().unwrap().iter().any(|i| {
            i["rgba"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_u64().unwrap() > 0)
        }));
        let target = preview.scene.targets[0].rect;
        assert!(!preview.pointer(550., 290.));
        assert!(preview.pointer(target.x + 4., target.y + 4.));
        assert_eq!(preview.host.as_ref().unwrap().state, vec![1.]);
        let after = frame(&mut preview);
        assert_ne!(before["paint"], after["paint"]);
        preview
            .compile(&SOURCE.replace("count + 1", "count + 2"), false)
            .unwrap();
        frame(&mut preview);
        assert!(preview.key("Enter", false));
        assert_eq!(preview.host.as_ref().unwrap().state, vec![3.]);
        let good = frame(&mut preview);
        assert!(
            preview
                .compile("export fn App() { invalid !!!", false)
                .is_err()
        );
        assert_eq!(frame(&mut preview), good);
        preview.compile(SOURCE, true).unwrap();
        assert_eq!(preview.host.as_ref().unwrap().state, vec![0.]);
        frame(&mut preview);
        assert!(preview.key("Tab", false));
        assert!(!preview.key("Tab", false));
    }
    #[test]
    fn scene_and_hit_testing_follow_the_viewport_and_style() {
        let mut preview = NativePreview::new();
        preview
            .compile(&SOURCE.replace(" w-96", ""), false)
            .unwrap();
        preview.frame(560., 300., 1.);
        let wide = preview.scene.targets[0].rect;
        preview.frame(320., 300., 2.);
        let narrow = preview.scene.targets[0].rect;
        assert!(narrow.width < wide.width);
        assert_eq!(wide.x, narrow.x);
        assert!(!preview.pointer(330., narrow.y + 2.));
    }
}
