use crate::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    rc::Rc,
};
#[derive(Clone, Debug, PartialEq)]
pub enum HostValue {
    Unit,
    Number(f64),
    Bool(bool),
    String(String),
    Strings(Vec<String>),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HostType {
    Unit,
    Number,
    Bool,
    String,
    Strings,
}
impl HostType {
    pub fn accepts(self, value: &HostValue) -> bool {
        matches!(
            (self, value),
            (Self::Unit, HostValue::Unit)
                | (Self::Number, HostValue::Number(_))
                | (Self::Bool, HostValue::Bool(_))
                | (Self::String, HostValue::String(_))
                | (Self::Strings, HostValue::Strings(_))
        )
    }
    pub fn source(self) -> &'static str {
        match self {
            Self::Unit => "void",
            Self::Number => "number",
            Self::Bool => "boolean",
            Self::String => "string",
            Self::Strings => "Array<string>",
        }
    }
}
pub type HostFuture = Pin<Box<dyn Future<Output = Result<HostValue>>>>;
pub enum HostReply {
    Ready(Result<HostValue>),
    Pending(HostFuture),
}
#[derive(Clone)]
pub struct HostOp {
    pub name: String,
    pub args: Vec<HostType>,
    pub result: HostType,
    pub asynchronous: bool,
    /// Operational failures are Result data. Protocol/capability faults remain
    /// VM errors. The declaration and dispatch share this output contract.
    pub result_channel: bool,
    pub capability: Option<String>,
    handler: Rc<dyn Fn(Vec<HostValue>) -> HostReply>,
}
impl HostOp {
    /// Declare `Result<T, string>` (or `Promise<Result<T, string>>`) output.
    pub fn with_result_channel(mut self) -> Self {
        self.result_channel = true;
        self
    }
    fn output_source(&self) -> String {
        let value = if self.result_channel {
            format!("Result<{}, string>", self.result.source())
        } else {
            self.result.source().to_owned()
        };
        if self.asynchronous {
            format!("Promise<{value}>")
        } else {
            value
        }
    }
    pub fn new(
        name: &str,
        args: Vec<HostType>,
        result: HostType,
        asynchronous: bool,
        capability: Option<&str>,
        handler: impl Fn(Vec<HostValue>) -> HostReply + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            args,
            result,
            asynchronous,
            result_channel: false,
            capability: capability.map(str::to_owned),
            handler: Rc::new(handler),
        }
    }
}
#[derive(Default, Clone)]
pub struct Hosts {
    operations: BTreeMap<String, HostOp>,
    grants: BTreeSet<String>,
}
impl Hosts {
    pub fn register(&mut self, op: HostOp) -> Result<()> {
        if op.name.is_empty()
            || !op
                .name
                .chars()
                .enumerate()
                .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
        {
            return Err("invalid host operation name".into());
        }
        if self.operations.contains_key(&op.name) {
            return Err("duplicate host operation".into());
        }
        self.operations.insert(op.name.clone(), op);
        Ok(())
    }
    pub fn grant(&mut self, capability: &str) {
        self.grants.insert(capability.into());
    }
    pub fn operation(&self, name: &str) -> Result<&HostOp> {
        self.operations
            .get(name)
            .ok_or_else(|| format!("unknown host operation: {name}"))
    }
    pub(crate) fn call(
        &self,
        name: &str,
        args: Vec<HostValue>,
    ) -> Result<(HostReply, HostType, bool, bool)> {
        let op = self.operation(name)?;
        if let Some(cap) = &op.capability
            && !self.grants.contains(cap)
        {
            return Err(format!("permission denied: {cap}"));
        }
        if args.len() != op.args.len() || !op.args.iter().zip(&args).all(|(t, v)| t.accepts(v)) {
            return Err(format!("invalid arguments for host operation {name}"));
        }
        let reply = (op.handler)(args);
        if !op.asynchronous && matches!(reply, HostReply::Pending(_)) {
            return Err(format!(
                "synchronous host operation {name} returned a future"
            ));
        }
        Ok((reply, op.result, op.asynchronous, op.result_channel))
    }
    /// The compiler's imported module signatures come from the same registry as dispatch.
    pub fn declarations(&self) -> String {
        self.operations
            .values()
            .map(|op| {
                let args = op
                    .args
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("arg{i}: {}", t.source()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let value = match op.result {
                    HostType::Unit => "",
                    HostType::Number => "return 0;",
                    HostType::Bool => "return false;",
                    HostType::Strings => "return [];",
                    HostType::String => "return \"\";",
                };
                let value = if op.result_channel {
                    // Declaration bodies are parsed only to collect signatures;
                    // dispatch invokes the registered Rust handler. Keep this
                    // body valid even for the void payload type.
                    match op.result {
                        HostType::Unit => "return Ok((fn() void {})());".to_owned(),
                        _ => format!(
                            "return Ok({});",
                            value.trim_start_matches("return ").trim_end_matches(';')
                        ),
                    }
                } else {
                    value.to_owned()
                };
                format!(
                    "export {}fn {}({args}) {} {{ {value} }}\n",
                    if op.asynchronous { "async " } else { "" },
                    op.name,
                    op.output_source()
                )
            })
            .collect()
    }
}
