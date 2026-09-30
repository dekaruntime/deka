//! Development interpreter. This crate is never linked into generated applications.
use deka_native_ir::{Node, Program};
use deka_native_ui::program::{ProgramApp, validate};
use deka_native_ui::{Application, Reload};
use std::{
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

pub struct DevApp {
    app: ProgramApp,
    updates: Option<Receiver<Result<Program, String>>>,
    stop: Arc<AtomicBool>,
}
impl DevApp {
    pub fn new(program: Program) -> Result<Self, String> {
        validate(&program)?;
        Ok(Self {
            app: ProgramApp::new(program)?,
            updates: None,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn replace(&mut self, program: Program) -> Result<Reload, String> {
        self.app.replace(program)
    }
    pub fn watch(source: &Path, compiler: &Path) -> Result<Self, String> {
        let initial_source = std::fs::read(source).map_err(|e| e.to_string())?;
        let mut app = Self::new(compile_file(source, compiler)?)?;
        let (tx, rx) = mpsc::channel();
        app.updates = Some(rx);
        let source = source.to_owned();
        let compiler = compiler.to_owned();
        let stop = app.stop.clone();
        thread::spawn(move || {
            let mut last = initial_source;
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(100));
                match std::fs::read(&source) {
                    Ok(bytes) if bytes != last => {
                        last = bytes;
                        if tx.send(compile_file(&source, &compiler)).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok(app)
    }
}
pub fn compile_file(source: &Path, compiler: &Path) -> Result<Program, String> {
    let output = Command::new(compiler)
        .arg("program")
        .arg(source)
        .output()
        .map_err(|e| format!("run {}: {e}", compiler.display()))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let program = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("invalid native program: {e}"))?;
    validate(&program)?;
    Ok(program)
}
impl Application for DevApp {
    fn initial_state(&self) -> Vec<f64> {
        self.app.initial_state()
    }
    fn render(&self, state: &[f64]) -> Node {
        self.app.render(state)
    }
    fn event(&self, handler: usize, state: &mut [f64]) {
        self.app.event(handler, state);
    }
    fn poll_reload(&mut self) -> Reload {
        let mut latest = None;
        if let Some(rx) = &self.updates {
            for result in rx.try_iter() {
                latest = Some(result);
            }
        }
        match latest {
            Some(Ok(program)) => match self.replace(program) {
                Ok(reload) => {
                    eprintln!("native: source reloaded");
                    reload
                }
                Err(error) => {
                    eprintln!("native: keeping last working program: {error}");
                    Reload::Unchanged
                }
            },
            Some(Err(error)) => {
                eprintln!("native: keeping last working program: {error}");
                Reload::Unchanged
            }
            None => Reload::Unchanged,
        }
    }
    fn live(&self) -> bool {
        self.updates.is_some()
    }
}

impl Drop for DevApp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
