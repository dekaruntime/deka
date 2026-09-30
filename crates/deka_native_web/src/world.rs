use wasm_bindgen::prelude::*;
/// Experimental shared world; no browser layout or JavaScript movement simulation.
#[wasm_bindgen]
pub struct PortfolioWorld {
    world: deka_native_ui::world::World,
}
impl Default for PortfolioWorld {
    fn default() -> Self {
        Self::new()
    }
}
#[wasm_bindgen]
impl PortfolioWorld {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            world: deka_native_ui::world::World::new(),
        }
    }
    pub fn start(&mut self) {
        self.world.start();
    }
    pub fn key(&mut self, key: &str, down: bool) -> bool {
        self.world.key(key, down)
    }
    pub fn interact(&mut self) {
        self.world.interact();
    }
    pub fn blur(&mut self) {
        self.world.blur();
    }
    pub fn set_muted(&mut self, muted: bool) {
        self.world.set_muted(muted);
    }
    pub fn frame(
        &mut self,
        width: f32,
        height: f32,
        milliseconds: f64,
        reduced_motion: bool,
    ) -> String {
        serde_json::to_string(
            &self
                .world
                .frame(width, height, milliseconds, reduced_motion),
        )
        .expect("finite world scene")
    }
    pub fn snapshot(&self) -> String {
        serde_json::to_string(&self.world.snapshot()).expect("finite world state")
    }
    pub fn sounds(&mut self) -> Vec<u8> {
        self.world.drain_sounds()
    }
    pub fn audio_samples(id: u8) -> Vec<f32> {
        deka_native_ui::world::audio::samples(id)
    }
}
