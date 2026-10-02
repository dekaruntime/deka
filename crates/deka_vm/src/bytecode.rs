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
    /// Like `Load`, but errors with `message` when the cell still holds the
    /// `Uninitialized` sentinel — a read of an export whose module has not
    /// finished initializing across an import cycle (deka#1206).
    LoadChecked {
        slot: usize,
        message: String,
    },
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    Equal,
    NotEqual,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Neg,
    Not,
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
    /// `string(x)`: number and bool widen to text, string passes through.
    ToString,
    /// `toNumber(x)`: bool widens to 1/0, number passes through.
    ToNumber,
    /// `panic(message)`: stop the program with the message as the error.
    Panic,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Literal {
    Unit,
    /// Sentinel stored into pre-allocated export slots of import-cycle
    /// members; only `LoadChecked` reads it without erroring.
    Uninitialized,
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
                    Op::LoadChecked { slot, .. } if *slot >= f.locals => {
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
