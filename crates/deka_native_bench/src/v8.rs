use crate::Backend;

pub struct V8Backend {
    js: deno_core::JsRuntime,
}
impl Default for V8Backend {
    fn default() -> Self {
        Self::new()
    }
}
impl V8Backend {
    pub fn new() -> Self {
        let mut js = deno_core::JsRuntime::new(Default::default());
        js.execute_script("backend.js", "globalThis.increment = n => n + 1;")
            .expect("initialize JS backend");
        Self { js }
    }
}
impl Backend for V8Backend {
    fn increment(&mut self, value: f64) -> f64 {
        let result = self
            .js
            .execute_script("call.js", format!("increment({value})"))
            .expect("call JS backend");
        deno_core::scope!(scope, self.js);
        let value = deno_core::v8::Local::new(scope, result);
        value.number_value(scope).expect("numeric backend result")
    }
}
