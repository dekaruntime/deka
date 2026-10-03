use crate::{HostCallback, HostContext, Result};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    fmt,
    future::Future,
    pin::Pin,
    rc::Rc,
};
/// An opaque resource remains owned by Rust. Language aliases retain the same
/// resource; tracing GC drops their references when no longer reachable.
#[derive(Clone)]
pub struct HostHandle {
    name: String,
    resource: Rc<dyn Any>,
}
impl HostHandle {
    pub fn new<T: Any>(name: &str, resource: T) -> Self {
        Self {
            name: name.into(),
            resource: Rc::new(resource),
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.resource.downcast_ref()
    }
}
impl fmt::Debug for HostHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("HostHandle").field(&self.name).finish()
    }
}
impl PartialEq for HostHandle {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && Rc::ptr_eq(&self.resource, &other.resource)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum HostValue {
    Unit,
    Number(f64),
    Bool(bool),
    String(String),
    Strings(Vec<String>),
    List(Vec<HostValue>),
    Record(BTreeMap<String, HostValue>),
    Bytes(Vec<u8>),
    Handle(HostHandle),
    Callback(HostCallback),
}
#[derive(Clone, Debug, PartialEq)]
pub enum HostType {
    Unit,
    Number,
    Bool,
    String,
    Strings,
    List(Box<HostType>),
    Record(BTreeMap<String, HostType>),
    Bytes,
    Handle(String),
    Callback,
}
impl HostType {
    pub fn accepts(&self, value: &HostValue) -> bool {
        match (self, value) {
            (Self::Unit, HostValue::Unit)
            | (Self::Number, HostValue::Number(_))
            | (Self::Bool, HostValue::Bool(_))
            | (Self::String, HostValue::String(_))
            | (Self::Strings, HostValue::Strings(_))
            | (Self::Bytes, HostValue::Bytes(_))
            | (Self::Callback, HostValue::Callback(_)) => true,
            (Self::Strings, HostValue::List(items)) => {
                items.iter().all(|v| Self::String.accepts(v))
            }
            (Self::List(item), HostValue::List(items)) => items.iter().all(|v| item.accepts(v)),
            (Self::List(item), HostValue::Strings(_)) => **item == Self::String,
            (Self::Record(fields), HostValue::Record(values)) => fields
                .iter()
                .all(|(name, ty)| values.get(name).is_some_and(|v| ty.accepts(v))),
            (Self::Handle(name), HostValue::Handle(handle)) => name == handle.name(),
            _ => false,
        }
    }
    pub fn source(&self) -> String {
        match self {
            Self::Unit => "void".into(),
            Self::Number => "number".into(),
            Self::Bool => "boolean".into(),
            Self::String => "string".into(),
            Self::Strings => "Array<string>".into(),
            Self::Bytes => "bytes".into(),
            Self::Callback => "(fn() void) | (fn() Promise<void>)".into(),
            Self::List(item) => format!("Array<{}>", item.source()),
            Self::Record(fields) => format!(
                "{{{}}}",
                fields
                    .iter()
                    .map(|(name, ty)| { format!("{name}: {}", ty.source()) })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Handle(name) => name.clone(),
        }
    }
    fn validate(&self, handles: &mut BTreeSet<String>, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err("host type nesting limit exceeded".into());
        }
        match self {
            Self::Record(fields) => {
                for (name, ty) in fields {
                    if !identifier(name) {
                        return Err("invalid host record field name".into());
                    }
                    ty.validate(handles, depth + 1)?;
                }
            }
            Self::List(item) => item.validate(handles, depth + 1)?,
            Self::Handle(name) => {
                // Uppercase names distinguish host brands from built-in primitives.
                if !identifier(name)
                    || !name.starts_with(char::is_uppercase)
                    || matches!(
                        name.as_str(),
                        "Option"
                            | "Result"
                            | "Promise"
                            | "Exception"
                            | "Array"
                            | "Type"
                            | "Component"
                            | "ReactNode"
                            | "JsError"
                            | "Error"
                            | "SyntaxError"
                            | "TypeError"
                            | "RangeError"
                            | "JsValue"
                            | "OpenContext"
                            | "Context"
                    )
                {
                    return Err("invalid opaque host type name".into());
                }
                handles.insert(name.clone());
            }
            _ => {}
        }
        Ok(())
    }
    // Preserve the legacy Strings Rust API while all VM lists use one recursive wire path.
    fn normalize(&self, value: HostValue) -> HostValue {
        match (self, value) {
            (Self::Strings, HostValue::List(items)) => HostValue::Strings(
                items
                    .into_iter()
                    .map(|v| {
                        let HostValue::String(s) = v else {
                            unreachable!("validated string list")
                        };
                        s
                    })
                    .collect(),
            ),
            (Self::List(item), HostValue::List(items)) => {
                HostValue::List(items.into_iter().map(|v| item.normalize(v)).collect())
            }
            (Self::List(_), HostValue::Strings(items)) => {
                HostValue::List(items.into_iter().map(HostValue::String).collect())
            }
            (Self::Record(fields), HostValue::Record(values)) => HostValue::Record(
                values
                    .into_iter()
                    .map(|(name, value)| {
                        let value = match fields.get(&name) {
                            Some(ty) => ty.normalize(value),
                            None => value,
                        };
                        (name, value)
                    })
                    .collect(),
            ),
            (_, value) => value,
        }
    }
}
fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

pub type HostFuture = Pin<Box<dyn Future<Output = Result<HostValue>>>>;
pub enum HostReply {
    Ready(Result<HostValue>),
    Pending(HostFuture),
}
type HostHandler = dyn Fn(&HostContext, Vec<HostValue>) -> HostReply;

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
    pub global: bool,
    handler: Rc<HostHandler>,
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
        Self::with_context(
            name,
            args,
            result,
            asynchronous,
            capability,
            move |_, args| handler(args),
        )
    }
    pub fn with_global_binding(mut self) -> Self {
        self.global = true;
        self
    }
    pub fn with_context(
        name: &str,
        args: Vec<HostType>,
        result: HostType,
        asynchronous: bool,
        capability: Option<&str>,
        handler: impl Fn(&HostContext, Vec<HostValue>) -> HostReply + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            args,
            result,
            asynchronous,
            result_channel: false,
            global: false,
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
        for ty in op.args.iter().chain(std::iter::once(&op.result)) {
            ty.validate(&mut BTreeSet::new(), 0)?;
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
        context: &HostContext,
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
        let args = op
            .args
            .iter()
            .zip(args)
            .map(|(ty, value)| ty.normalize(value))
            .collect();
        let reply = (op.handler)(context, args);
        if !op.asynchronous && matches!(reply, HostReply::Pending(_)) {
            return Err(format!(
                "synchronous host operation {name} returned a future"
            ));
        }
        Ok((reply, op.result.clone(), op.asynchronous, op.result_channel))
    }
    pub(crate) fn declarations_names_and_arities(&self) -> BTreeMap<String, usize> {
        self.operations
            .values()
            .map(|op| (op.name.clone(), op.args.len()))
            .collect()
    }
    pub fn globals(&self) -> impl Iterator<Item = &HostOp> {
        self.operations.values().filter(|op| op.global)
    }
    /// The compiler's imported module signatures come from the same registry as dispatch.
    pub fn declarations(&self) -> String {
        let mut handles = BTreeSet::new();
        for op in self.operations.values() {
            for ty in op.args.iter().chain(std::iter::once(&op.result)) {
                // Registration already validated every schema.
                ty.validate(&mut handles, 0)
                    .expect("registered host schema");
            }
        }
        let mut source = handles
            .into_iter()
            .map(|name| format!("export opaque type {name};\n"))
            .collect::<String>();
        for op in self.operations.values() {
            let args = op
                .args
                .iter()
                .enumerate()
                .map(|(i, t)| format!("arg{i}: {}", t.source()))
                .collect::<Vec<_>>()
                .join(", ");
            // Host failures are Result data or VM protocol faults, never language
            // Throw. A total declaration expresses that without a fake body.
            source.push_str(&format!(
                "export total fn {}({args}) {};\n",
                op.name,
                op.output_source()
            ));
        }
        source
    }
}
