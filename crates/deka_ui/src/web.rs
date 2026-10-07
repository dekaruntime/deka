//! Browser sessions use the same retained tree, events and renderer as desktop.
use crate::{Event, UiApp};
use deka_native_ui::scene::{Renderer, Scene};
use std::collections::HashSet;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct BrowserApp {
    app: UiApp,
    renderer: Renderer,
    scene: Scene,
    focus: Option<String>,
    sent_images: HashSet<String>,
}
impl BrowserApp {
    pub fn new(app: UiApp) -> Self {
        Self {
            app,
            renderer: Renderer::new(),
            scene: Scene::default(),
            focus: None,
            sent_images: HashSet::new(),
        }
    }
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl BrowserApp {
    pub fn frame_at(
        &mut self,
        width: f32,
        height: f32,
        scale: f32,
        milliseconds: f64,
        reduced: bool,
    ) -> Result<String, String> {
        let tree = self.app.tree();
        self.scene = self
            .renderer
            .render_at(&tree, width, height, scale, milliseconds, reduced);
        let errors = self.app.take_errors();
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        // The renderer retains glyph textures by id. Send each bitmap only once
        // while it is live; keep the full scene for Rust hit testing.
        let active: HashSet<_> = self
            .scene
            .images
            .iter()
            .map(|image| image.id.clone())
            .collect();
        self.sent_images.retain(|id| active.contains(id));
        let images = std::mem::take(&mut self.scene.images);
        self.scene.images = images
            .iter()
            .filter(|image| self.sent_images.insert(image.id.clone()))
            .cloned()
            .collect();
        #[derive(serde::Serialize)]
        struct Frame<'a> {
            #[serde(flatten)]
            scene: &'a Scene,
            image_ids: Vec<&'a str>,
        }
        let frame = Frame {
            scene: &self.scene,
            image_ids: images.iter().map(|image| image.id.as_str()).collect(),
        };
        let result = serde_json::to_string(&frame).map_err(|e| e.to_string());
        self.scene.images = images;
        result
    }
    pub fn pointer(&mut self, x: f32, y: f32) -> bool {
        if let Some(target) = self.scene.hit(x, y) {
            self.focus = Some(target.id.clone());
            return self.app.dispatch(target.handler);
        }
        self.focus = self
            .scene
            .nodes
            .iter()
            .rev()
            .find(|node| node.rect.contains(x, y) && node.clip.contains(x, y))
            .map(|node| node.id.clone());
        false
    }
    /// Tab follows rendered controls; boundaries let focus leave the canvas.
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
        if let Some(id) = &self.focus {
            if self.app.dispatch_to(id, Event::KeyDown(key.into())) {
                return true;
            }
            if matches!(key, "Enter" | " ") {
                return self.app.dispatch_to(id, Event::Click);
            }
        }
        false
    }
    /// The browser's native text field supplies the complete edited value.
    pub fn input(&self, node_id: &str, value: &str) -> bool {
        self.app.dispatch_to(node_id, Event::Input(value.into()))
    }
    pub fn key_to(&self, node_id: &str, key: &str) -> bool {
        self.app.dispatch_to(node_id, Event::KeyDown(key.into()))
    }
    pub fn blur(&mut self) {
        self.focus = None;
    }
    pub fn focus(&self) -> Option<String> {
        self.focus.clone()
    }
    pub fn inputs(&self) -> Result<String, String> {
        use deka_native_ui::Application;
        let inputs:Vec<_> = self.app.text_controls().into_iter().map(|control| serde_json::json!({
            "id":control.id,"value":control.value,"tag":if control.multiline {"textarea"} else {"input"},"placeholder":control.placeholder,"controlled":control.controlled
        })).collect();
        serde_json::to_string(&inputs).map_err(|e| e.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
#[cfg_attr(feature = "web-test", wasm_bindgen(module = "/web/test-host.js"))]
#[cfg_attr(not(feature = "web-test"), wasm_bindgen(module = "/web/host.js"))]
extern "C" {
    #[wasm_bindgen(catch, js_name = mount)]
    fn mount(app: BrowserApp, canvas: &JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = reportPanic)]
    fn report_panic(message: &str);
    #[wasm_bindgen(js_name = unmount)]
    fn unmount(handle: &JsValue);
}
/// Owns browser listeners, animation, the renderer and the app's reactive scope.
#[cfg(target_arch = "wasm32")]
pub struct WebHandle(JsValue);
#[cfg(target_arch = "wasm32")]
impl Drop for WebHandle {
    fn drop(&mut self) {
        unmount(&self.0);
    }
}
#[cfg(target_arch = "wasm32")]
pub fn mount_app(app: UiApp, canvas: &JsValue) -> Result<WebHandle, JsValue> {
    // No extra dependency is needed to turn Rust panic payloads into readable
    // browser diagnostics. The hook runs before wasm reports "unreachable".
    static PANIC_HOOK: std::sync::Once = std::sync::Once::new();
    PANIC_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| report_panic(&info.to_string())));
    });
    mount(BrowserApp::new(app), canvas).map(WebHandle)
}
/// Launch precompiled Rust into an existing HTML canvas. Drop the handle to stop.
#[cfg(target_arch = "wasm32")]
pub fn launch<A, M>(app: A, canvas: &JsValue) -> Result<WebHandle, JsValue>
where
    A: crate::view::BuildApp<M>,
{
    mount_app(UiApp::new(app), canvas)
}
