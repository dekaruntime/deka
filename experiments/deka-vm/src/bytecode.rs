use serde::{Deserialize, Serialize};

// Stack instruction family recovered from PHPX 61c262e8's vm/opcode.rs.
// Operands and closures are redesigned for the checked DekaScript slice.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Op {
    Const(Literal),
    Pop,
    Dup,
    Load(usize),
    Store(usize),
    Add,
    Sub,
    Mul,
    Div,
    Less,
    Equal,
    Jump(usize),
    JumpIfFalse(usize),
    Closure {
        function: usize,
        captures: Vec<usize>,
    },
    Call(usize),
    Host {
        operation: String,
        arguments: usize,
    },
    Await,
    Return,
    List(usize),
    ListAppend,
    ListHas,
    Index,
    Record(Vec<String>),
    Field(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Literal {
    Unit,
    Number(f64),
    Bool(bool),
    String(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub parameters: usize,
    pub captures: usize,
    pub locals: usize,
    pub asynchronous: bool,
    pub code: Vec<Op>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Program {
    pub version: u32,
    pub functions: Vec<Function>,
}
impl Program {
    pub fn validate(&self) -> crate::Result<()> {
        if self.version != 1 || self.functions.is_empty() {
            return Err("unsupported or empty bytecode".into());
        }
        for f in &self.functions {
            if f.parameters.saturating_add(f.captures) > f.locals || f.locals > 65536 {
                return Err("invalid local count".into());
            }
            for op in &f.code {
                match op {
                    Op::Load(i) | Op::Store(i) if *i >= f.locals => {
                        return Err("invalid local operand".into());
                    }
                    Op::Jump(i) | Op::JumpIfFalse(i) if *i >= f.code.len() => {
                        return Err("invalid jump".into());
                    }
                    Op::Closure { function, captures } => {
                        let target = self
                            .functions
                            .get(*function)
                            .ok_or("invalid function operand")?;
                        if captures.len() != target.captures
                            || captures.iter().any(|i| *i >= f.locals)
                        {
                            return Err("invalid capture operands".into());
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}
