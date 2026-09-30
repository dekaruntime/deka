use crate::Backend;
use rquickjs::{Context, Runtime};

/// A live QuickJS heap retained for the lifetime of the native application.
/// Binding-owned handles manage reference counts; no manual retain/release here.
pub struct QuickJsBackend {
    context: Context,
    _runtime: Runtime,
}
impl QuickJsBackend {
    pub fn new() -> rquickjs::Result<Self> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| ctx.eval::<(), _>("globalThis.increment = n => n + 1;"))?;
        Ok(Self {
            context,
            _runtime: runtime,
        })
    }
}
impl Backend for QuickJsBackend {
    fn increment(&mut self, value: f64) -> f64 {
        // Match the V8 benchmark's source evaluation on every event.
        self.context
            .with(|ctx| ctx.eval(format!("increment({value})")))
            .expect("numeric QuickJS backend result")
    }
}
