//! Portable development interpreter used by the native watcher and browser host.
use crate::{Application, Reload};
use deka_native_ir::{FORMAT_VERSION, Node, Number, Program, Template, Text};

pub struct ProgramApp {
    program: Program,
}
impl ProgramApp {
    pub fn new(program: Program) -> Result<Self, String> {
        validate(&program)?;
        Ok(Self { program })
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
}
impl Application for ProgramApp {
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
pub fn validate(program: &Program) -> Result<(), String> {
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
