//! Development interpreter. This crate is never linked into generated applications.
use deka_native_ir::{FORMAT_VERSION, Node, Number, Program, Template, Text};
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
    program: Program,
    updates: Option<Receiver<Result<Program, String>>>,
    stop: Arc<AtomicBool>,
}
impl DevApp {
    pub fn new(program: Program) -> Result<Self, String> {
        validate(&program)?;
        Ok(Self {
            program,
            updates: None,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn replace(&mut self, program: Program) -> Result<Reload, String> {
        validate(&program)?;
        let compatible = self.program.component == program.component
            && self
                .program
                .states
                .iter()
                .map(|s| &s.name)
                .eq(program.states.iter().map(|s| &s.name));
        self.program = program;
        Ok(if compatible {
            Reload::Preserve
        } else {
            Reload::Reset
        })
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
        self.program.states.iter().map(|s| s.initial).collect()
    }
    fn render(&self, state: &[f64]) -> Node {
        render(&self.program.root, state)
    }
    fn event(&self, handler: usize, state: &mut [f64]) {
        if let Some(update) = self.program.handlers.get(handler) {
            state[update.state] = number(&update.value, state);
        }
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
fn number(expr: &Number, state: &[f64]) -> f64 {
    match expr {
        Number::Literal(n) => *n,
        Number::State(i) => state[*i],
        Number::Add(a, b) => number(a, state) + number(b, state),
        Number::Sub(a, b) => number(a, state) - number(b, state),
        Number::Mul(a, b) => number(a, state) * number(b, state),
    }
}
fn render(template: &Template, state: &[f64]) -> Node {
    Node {
        id: template.id.clone(),
        style: template.style.clone(),
        on_click: template.on_click,
        text: template.text.as_ref().map(|text| match text {
            Text::Literal(s) => s.clone(),
            Text::Number(n) => number(n, state).to_string(),
        }),
        children: template
            .children
            .iter()
            .map(|child| render(child, state))
            .collect(),
    }
}
fn validate(program: &Program) -> Result<(), String> {
    if program.format != FORMAT_VERSION {
        return Err(
            "native program format mismatch; rebuild the compiler and host together".into(),
        );
    }
    if program.states.iter().any(|s| !s.initial.is_finite()) {
        return Err("non-finite initial state".into());
    }
    fn check_number(expr: &Number, count: usize) -> bool {
        match expr {
            Number::Literal(n) => n.is_finite(),
            Number::State(i) => *i < count,
            Number::Add(a, b) | Number::Sub(a, b) | Number::Mul(a, b) => {
                check_number(a, count) && check_number(b, count)
            }
        }
    }
    fn check_node(node: &Template, program: &Program) -> bool {
        node.on_click.is_none_or(|i| i < program.handlers.len())
            && node.text.as_ref().is_none_or(|t| match t {
                Text::Literal(_) => true,
                Text::Number(n) => check_number(n, program.states.len()),
            })
            && node.children.iter().all(|c| check_node(c, program))
    }
    if !check_node(&program.root, program)
        || program.handlers.iter().any(|h| {
            h.state >= program.states.len() || !check_number(&h.value, program.states.len())
        })
    {
        return Err("invalid native state or handler reference".into());
    }
    Ok(())
}

impl Drop for DevApp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
