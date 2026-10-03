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
    /// Replace a local's cell with a fresh one holding unit. Emitted at
    /// declaration sites inside loops so closures capture the current
    /// iteration's value rather than aliasing the next iteration's.
    Rebind(usize),
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
    /// Pop a value and jump when it is unit. Used by default parameters:
    /// an omitted argument arrives as unit.
    JumpIfUnit(usize),
    Closure {
        function: usize,
        captures: Vec<usize>,
    },
    Call(usize),
    /// Pop a props value, pop a component closure, and call the component
    /// with the props (or with no argument when it declares no parameter).
    /// The props' nested `children` become the callee's default-slot content.
    ComponentCall,
    /// Push the current frame's default-slot content (unit when none).
    Slot,
    /// Pop the arguments, pop the receiver record, and call its `$<name>`
    /// member (an attached method, which receives the record as its first
    /// argument) or its `<name>` member (a field holding a function, called
    /// plainly). Backs interface method calls, where the concrete type is
    /// only known at run time.
    MethodCall {
        name: String,
        argc: usize,
    },
    Host {
        operation: String,
        arguments: usize,
    },
    /// Native promise combinators consume a list of VM promise handles.
    PromiseJoin(PromiseJoin),
    Await,
    /// Frame-local handler; saves operand depth and receives the thrown payload.
    Handler(usize),
    EndHandler,
    Throw,
    Return,
    List(usize),
    ListAppend,
    ListHas,
    /// Pop a source list, pop a target list, push target with the source's
    /// items appended. Backs `[...xs]` in list literals.
    ListExtend,
    Index,
    /// Pop value, pop object, set the object's field to the value, push the
    /// value. Backs `obj.field = v` through a `let` binding; const-ness is
    /// the typechecker's job.
    FieldSet(String),
    /// Pop value, pop index, pop object, set the element, push the value.
    /// Backs `list[i] = v`; strings are immutable and reject it.
    IndexSet,
    /// In-place list operations behind the mutating built-ins. The receiver
    /// list is under the arguments on the stack.
    ListMut(ListMut),
    Record(Vec<String>),
    /// A nominal struct with canonical nested embedded values.
    Struct {
        name: String,
        /// Module/declaration identity is independent of display names and aliases.
        #[serde(default)]
        identity: Option<String>,
        fields: Vec<String>,
        embeds: Vec<String>,
    },
    /// Like `Record`, but builds component props: attribute values are
    /// zero-argument getter closures that `Field` calls on every read.
    Props(Vec<String>),
    /// Pop a source record, pop a target record, push target with the
    /// source's fields merged over it. Backs `{...obj}` in object literals.
    RecordExtend,
    Field(String),
    /// Like `Field`, but a record without the key stays itself. Backs the
    /// embed-path walk in a promoted method call: a literal may nest the
    /// embedded record under its type name or carry its fields flat.
    FieldOrSelf(String),
    /// `string(x)`: number and bool widen to text, string passes through.
    ToString,
    /// Runtime identity is distinct from the declared signature.
    GetType,
    Descriptor(TypeDescriptor),
    /// Convert a checked value using its concrete schema.
    JsonStringify(crate::JsonShape),
    /// Pop a struct method table, then JSON text; push a nominal Result.
    JsonParse(crate::JsonShape),
    /// Convert Result<string,string> from a consumed host body with the same
    /// schema and nominal factories as JSON.parse; propagate read errors.
    JsonParseResult(crate::JsonShape),
    Newtype(String),
    Enum {
        name: String,
        case: String,
        index: usize,
        payload: bool,
    },
    /// Nominal pattern predicates consume a value and produce a bool, without
    /// reading payload fields until a case has matched.
    MatchEnum {
        name: Option<String>,
        case: String,
    },
    MatchType(TypeDescriptor),
    MatchStruct(String),
    MatchTuple(usize),
    /// Scalar pattern equality returns false for a different value kind.
    MatchEqual,
    /// `toNumber(x)`: bool widens to 1/0, number passes through.
    ToNumber,
    /// `panic(message)`: stop the program with the message as the error.
    Panic,
}
/// One catalog supplies native namespace names, signatures and opcodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromiseJoin {
    All,
    Race,
}
impl PromiseJoin {
    pub const ALL: [Self; 2] = [Self::All, Self::Race];
    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Race => "race",
        }
    }
    #[cfg(feature = "compiler")]
    pub(crate) fn declaration(self) -> String {
        let ret = match self {
            Self::All => "Array<T>",
            Self::Race => "T",
        };
        format!(
            "export fn __promise_{}<T>(values: Array<Promise<T>>) Promise<{ret}> {{}}\n",
            self.name()
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TypeDescriptor {
    pub kind: String,
    pub name: String,
}
impl TypeDescriptor {
    pub fn new(kind: &str, name: &str) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
        }
    }
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
/// Mutating list built-ins. Argument order on the stack matches the declared
/// signatures; the result replaces them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ListMut {
    /// `push(v)`: append, result the new length.
    Push,
    /// `pop()`: remove the last element, result it (unit when empty).
    Pop,
    /// `shift()`: remove the first element, result it (unit when empty).
    Shift,
    /// `unshift(v)`: prepend, result the new length.
    Unshift,
    /// `splice(start, deleteCount)`: remove a range, result the removed list.
    Splice,
    /// `sort()`: order numbers ascending or strings lexicographically,
    /// result the same list.
    Sort,
    /// `reverse()`: reverse in place, result the same list.
    Reverse,
    /// `fill(v, start)`: overwrite from `start` to the end, result the list.
    Fill,
    /// `copyWithin(target, start)`: copy `start..` over `target..`, result
    /// the list.
    CopyWithin,
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
                    Op::Load(i) | Op::Store(i) | Op::Rebind(i) if *i >= f.locals => {
                        return Err("invalid local operand".into());
                    }
                    Op::LoadChecked { slot, .. } if *slot >= f.locals => {
                        return Err("invalid local operand".into());
                    }
                    Op::Jump(i) | Op::JumpIfFalse(i) | Op::JumpIfUnit(i) | Op::Handler(i)
                        if *i >= f.code.len() =>
                    {
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
