//! Persistent Deka isolate feeding the shared native renderer.
//! Component state and closures belong to React/V8; Rust owns only the render snapshot.
mod loader;
mod policy;
use deka_native_ir::{Node, Style};
use deka_native_ui::{Application, Reload};
use deno_core::{JsRuntime, ModuleSpecifier, RuntimeOptions, serde_v8, v8};
use pool::PhpxEsmLoader;
use security::security_context::{SecurityContext, set_security_context};
use serde::Deserialize;
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    task::{Context, Poll, Waker},
};

#[derive(Deserialize)]
struct Frame {
    version: u64,
    tree: WireNode,
}
#[derive(Deserialize)]
struct WireNode {
    id: String,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    classes: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    handler: Option<usize>,
    children: Vec<WireNode>,
}
impl WireNode {
    fn into_node(self) -> Result<Node, String> {
        let mut style = if self.text.is_some() {
            Style::default()
        } else {
            deka_native_ir::element_style(&self.tag)?
        };
        deka_native_ir::apply_classes(&mut style, &self.classes)?;
        Ok(Node {
            id: self.id,
            style,
            text: self.text,
            on_click: self.handler,
            children: self
                .children
                .into_iter()
                .map(Self::into_node)
                .collect::<Result<_, _>>()?,
        })
    }
}

pub struct Session {
    js: JsRuntime,
    executor: tokio::runtime::Runtime,
    security: SecurityContext,
    tree: Node,
    version: u64,
}
impl Session {
    /// `compiler` is the normal DSC CLI, not the restricted dsc-native compiler.
    pub fn open(
        project: &Path,
        entry: &Path,
        compiler: &Path,
        export: &str,
    ) -> Result<Self, String> {
        let project = project.canonicalize().map_err(|e| e.to_string())?;
        let compiler = compiler
            .canonicalize()
            .map_err(|e| format!("compiler {}: {e}", compiler.display()))?;
        let entry = entry.canonicalize().map_err(|e| e.to_string())?;
        if !entry.starts_with(&project) {
            return Err("native entry must be inside its explicit project root".into());
        }
        let policy_json = policy::resolve(&project)?;
        let security = SecurityContext {
            policy_json: Some(policy_json),
            no_prompt: true,
        };
        let _security = set_security_context(security.clone());
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let _enter = executor.enter();
        let loader = PhpxEsmLoader::new(project, entry.clone(), None, None, Some(compiler), false)
            .map_err(|e| e.to_string())?;
        let mut extensions = platform_server::extensions_for_php_server();
        extensions.push(pool::esm_loader::import_meta_ops::init());
        let mut js = JsRuntime::new(RuntimeOptions {
            extensions,
            module_loader: Some(Rc::new(loader::NativeLoader(loader.clone()))),
            ..Default::default()
        });
        js.op_state().borrow_mut().put(loader);
        js.execute_script("native-timers.js", include_str!("timers.js"))
            .map_err(|e| e.to_string())?;
        js.execute_script("deka-bootstrap.js", pool::bootstrap::bootstrap_source())
            .map_err(|e| e.to_string())?;
        executor.block_on(async {
            // Side module supplies the host renderer; the user's file remains the true main module.
            let id = js
                .load_side_es_module(&ModuleSpecifier::parse(loader::ADAPTER).unwrap())
                .await
                .map_err(|e| e.to_string())?;
            let ready = js.mod_evaluate(id);
            js.with_event_loop_promise(ready, Default::default())
                .await
                .map_err(|e| e.to_string())?;
            let specifier =
                ModuleSpecifier::from_file_path(&entry).map_err(|_| "invalid native entry path")?;
            let id = js
                .load_main_es_module(&specifier)
                .await
                .map_err(|e| e.to_string())?;
            let ready = js.mod_evaluate(id);
            js.with_event_loop_promise(ready, Default::default())
                .await
                .map_err(|e| e.to_string())?;
            let namespace = js.get_module_namespace(id).map_err(|e| e.to_string())?;
            deno_core::scope!(scope, &mut js);
            let ns = v8::Local::new(scope, namespace);
            let key = v8::String::new(scope, export).ok_or("invalid component export")?;
            let component = ns
                .get(scope, key.into())
                .ok_or("component export missing")?;
            let global = scope.get_current_context().global(scope);
            let name = v8::String::new(scope, "__dekaNativeComponent").unwrap();
            global.set(scope, name.into(), component);
            Ok::<(), String>(())
        })?;
        js.execute_script(
            "native-mount.js",
            "__dekaNative.mount(__dekaNativeComponent); delete globalThis.__dekaNativeComponent;",
        )
        .map_err(|e| e.to_string())?;
        let frame = Self::read_frame(&mut js)?;
        let tree = frame.tree.into_node()?;
        drop(_enter);
        Ok(Self {
            js,
            executor,
            security,
            tree,
            version: frame.version,
        })
    }
    fn read_frame(js: &mut JsRuntime) -> Result<Frame, String> {
        let value = js
            .execute_script("native-frame.js", "__dekaNative.frame()")
            .map_err(|e| e.to_string())?;
        deno_core::scope!(scope, js);
        let value = v8::Local::new(scope, value);
        serde_v8::from_v8(scope, value).map_err(|e| e.to_string())
    }
    pub fn tree(&self) -> &Node {
        &self.tree
    }
    /// Poll without waiting for long-lived timers or subscriptions to finish.
    pub fn pump(&mut self) -> Result<bool, String> {
        let _security = set_security_context(self.security.clone());
        let _enter = self.executor.enter();
        // One executor turn services Tokio timers; Deno keeps pending ops for the next turn.
        self.executor
            .block_on(async { tokio::task::yield_now().await });
        let mut cx = Context::from_waker(Waker::noop());
        if let Poll::Ready(Err(error)) = self.js.poll_event_loop(&mut cx, Default::default()) {
            return Err(error.to_string());
        }
        let frame = Self::read_frame(&mut self.js)?;
        let changed = frame.version != self.version;
        let tree = frame.tree.into_node()?;
        self.tree = tree;
        self.version = frame.version;
        Ok(changed)
    }
    pub fn click(&mut self, handler: usize) -> Result<(), String> {
        let _security = set_security_context(self.security.clone());
        let _enter = self.executor.enter();
        self.js
            .execute_script("native-click.js", format!("__dekaNative.click({handler})"))
            .map_err(|e| e.to_string())?;
        drop(_enter);
        self.pump()?;
        Ok(())
    }
    pub fn unmount(&mut self) -> Result<(), String> {
        let _security = set_security_context(self.security.clone());
        let _enter = self.executor.enter();
        self.js
            .execute_script("native-unmount.js", "__dekaNative.unmount()")
            .map_err(|e| e.to_string())?;
        drop(_enter);
        self.pump()?;
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.unmount();
    }
}

pub struct RuntimeApp(RefCell<Session>);
impl RuntimeApp {
    pub fn new(session: Session) -> Self {
        Self(RefCell::new(session))
    }
}
impl Application for RuntimeApp {
    fn initial_state(&self) -> Vec<f64> {
        vec![]
    }
    fn render(&self, _: &[f64]) -> Node {
        self.0.borrow().tree.clone()
    }
    fn event(&self, handler: usize, _: &mut [f64]) {
        if let Err(error) = self.0.borrow_mut().click(handler) {
            eprintln!("native runtime: {error}");
        }
    }
    fn poll_reload(&mut self) -> Reload {
        match self.0.get_mut().pump() {
            Ok(true) => Reload::Preserve,
            Ok(false) => Reload::Unchanged,
            Err(error) => {
                eprintln!("native runtime: {error}");
                Reload::Unchanged
            }
        }
    }
    fn live(&self) -> bool {
        true
    }
}
