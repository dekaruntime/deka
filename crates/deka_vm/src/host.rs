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
        Self::from_shared(name, Rc::new(resource))
    }
    /// Retain the caller's allocation directly so repeated handles to the same
    /// Rust-owned resource preserve identity without another ownership layer.
    pub fn from_shared<T: Any>(name: &str, resource: Rc<T>) -> Self {
        Self {
            name: name.into(),
            resource,
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
    Option(Option<Box<HostValue>>),
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
    Option(Box<HostType>),
    Tuple(Vec<HostType>),
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
            (Self::Option(item), HostValue::Option(value)) => {
                value.as_ref().is_none_or(|v| item.accepts(v))
            }
            (Self::Tuple(types), HostValue::List(items)) => {
                types.len() == items.len()
                    && types.iter().zip(items).all(|(ty, value)| ty.accepts(value))
            }
            (Self::List(item), HostValue::List(items)) => items.iter().all(|v| item.accepts(v)),
            (Self::List(item), HostValue::Strings(_)) => **item == Self::String,
            (Self::Record(fields), HostValue::Record(values)) => fields.iter().all(|(name, ty)| {
                values
                    .get(name)
                    .map_or(matches!(ty, Self::Option(_)), |v| ty.accepts(v))
            }),
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
            Self::Option(item) => format!("Option<{}>", item.source()),
            Self::Tuple(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(Self::source)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
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
            Self::List(item) | Self::Option(item) => item.validate(handles, depth + 1)?,
            Self::Tuple(items) => {
                for item in items {
                    item.validate(handles, depth + 1)?;
                }
            }
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
    pub(crate) fn normalize(&self, value: HostValue) -> HostValue {
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
            (Self::Option(item), HostValue::Option(value)) => {
                HostValue::Option(value.map(|value| Box::new(item.normalize(*value))))
            }
            (Self::Tuple(types), HostValue::List(items)) => HostValue::List(
                types
                    .iter()
                    .zip(items)
                    .map(|(ty, value)| ty.normalize(value))
                    .collect(),
            ),
            (Self::List(item), HostValue::List(items)) => {
                HostValue::List(items.into_iter().map(|v| item.normalize(v)).collect())
            }
            (Self::List(_), HostValue::Strings(items)) => {
                HostValue::List(items.into_iter().map(HostValue::String).collect())
            }
            (Self::Record(fields), HostValue::Record(mut values)) => {
                for (name, ty) in fields {
                    if matches!(ty, Self::Option(_)) {
                        values
                            .entry(name.clone())
                            .or_insert(HostValue::Option(None));
                    }
                }
                HostValue::Record(
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
                )
            }
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
    /// Operational failures are Result data. Protocol faults remain
    /// VM errors. The declaration and dispatch share this output contract.
    pub result_channel: bool,
    pub global: bool,
    /// A zero-argument Rust getter exposed as a typed ambient value.
    pub global_value: Option<String>,
    /// A typed static function exposed as a field of a global namespace.
    pub namespace: Option<(String, String)>,
    /// A declared method on an opaque Rust-owned receiver. Its first argument
    /// is the receiver; the same schema declares and dispatches the method.
    pub receiver_method: Option<(String, String)>,
    pub receiver_property: bool,
    /// Specialize this async string-body reader with the checked JSON schema.
    pub json_body: bool,
    defaults: Vec<HostValue>,
    handler: Rc<HostHandler>,
}
impl HostOp {
    /// Declare `Result<T, string>` (or `Promise<Result<T, string>>`) output.
    pub fn with_result_channel(mut self) -> Self {
        self.result_channel = true;
        self
    }
    pub fn with_defaults(mut self, defaults: Vec<HostValue>) -> Self {
        self.defaults = defaults;
        self
    }
    pub fn with_receiver_method(mut self, owner: &str, method: &str) -> Self {
        self.receiver_method = Some((owner.into(), method.into()));
        self.receiver_property = false;
        self
    }
    pub fn with_receiver_property(mut self, owner: &str, property: &str) -> Self {
        self.receiver_method = Some((owner.into(), property.into()));
        self.receiver_property = true;
        self
    }
    pub fn with_json_body(mut self) -> Self {
        self.json_body = true;
        self
    }
    fn parameters_source(&self, start: usize) -> String {
        self.args
            .iter()
            .enumerate()
            .skip(start)
            .map(|(i, ty)| {
                let default = i
                    .checked_sub(self.args.len() - self.defaults.len())
                    .and_then(|j| self.defaults.get(j))
                    .map(|value| {
                        format!(
                            " = {}",
                            default_source(value).expect("validated host default")
                        )
                    })
                    .unwrap_or_default();
                format!("arg{i}: {}{default}", ty.source())
            })
            .collect::<Vec<_>>()
            .join(", ")
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
        handler: impl Fn(Vec<HostValue>) -> HostReply + 'static,
    ) -> Self {
        Self::with_context(name, args, result, asynchronous, move |_, args| {
            handler(args)
        })
    }
    pub fn with_global_binding(mut self) -> Self {
        self.global = true;
        self
    }
    pub fn with_global_value_binding(mut self, name: &str) -> Self {
        self.global = true;
        self.global_value = Some(name.into());
        self
    }
    pub fn global_name(&self) -> &str {
        self.global_value.as_deref().unwrap_or(&self.name)
    }
    pub fn with_namespace_binding(mut self, namespace: &str, field: &str) -> Self {
        self.namespace = Some((namespace.into(), field.into()));
        self
    }
    pub fn with_context(
        name: &str,
        args: Vec<HostType>,
        result: HostType,
        asynchronous: bool,
        handler: impl Fn(&HostContext, Vec<HostValue>) -> HostReply + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            args,
            result,
            asynchronous,
            result_channel: false,
            global: false,
            global_value: None,
            namespace: None,
            receiver_method: None,
            receiver_property: false,
            json_body: false,
            defaults: Vec::new(),
            handler: Rc::new(handler),
        }
    }
}
#[derive(Default, Clone)]
pub struct Hosts {
    operations: BTreeMap<String, HostOp>,
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
        if op.global_value.as_ref().is_some_and(|name| {
            !identifier(name)
                || !op.global
                || !op.args.is_empty()
                || op.asynchronous
                || op.result_channel
                || !matches!(op.result, HostType::Handle(_))
                || op.receiver_method.is_some()
        }) {
            return Err("ambient host value needs a synchronous opaque getter".into());
        }
        if op.global
            && self
                .operations
                .values()
                .any(|old| old.global && old.global_name() == op.global_name())
        {
            return Err("duplicate ambient host name".into());
        }
        if let Some((namespace, field)) = &op.namespace
            && (!identifier(namespace)
                || !identifier(field)
                || op.global
                || op.receiver_method.is_some()
                || namespace == "Promise"
                || self.operations.values().any(|old| {
                    old.namespace.as_ref() == op.namespace.as_ref()
                        || (old.global && old.global_name() == namespace)
                }))
        {
            return Err("invalid or duplicate host namespace binding".into());
        }
        if op.global
            && self.operations.values().any(|old| {
                old.namespace
                    .as_ref()
                    .is_some_and(|(namespace, _)| namespace == op.global_name())
            })
        {
            return Err("host global conflicts with a namespace".into());
        }
        for ty in op.args.iter().chain(std::iter::once(&op.result)) {
            ty.validate(&mut BTreeSet::new(), 0)?;
        }
        if op.defaults.len() > op.args.len() {
            return Err("too many host default arguments".into());
        }
        for (ty, value) in op.args[op.args.len() - op.defaults.len()..]
            .iter()
            .zip(&op.defaults)
        {
            if !ty.accepts(value) || default_source(value).is_none() {
                return Err("invalid host default argument".into());
            }
        }
        if op.json_body
            && (op.receiver_method.is_none()
                || op.receiver_property
                || !op.asynchronous
                || !op.result_channel
                || op.result != HostType::String
                || op.args.len() != 1
                || !op.defaults.is_empty())
        {
            return Err("JSON body reader must be an async Result<string> receiver method".into());
        }
        if op.receiver_property && op.receiver_method.is_none() {
            return Err("host property needs a receiver".into());
        }
        if let Some((owner, method)) = &op.receiver_method
            && (!identifier(method)
                || op.args.first() != Some(&HostType::Handle(owner.clone()))
                || (op.receiver_property && (op.args.len() != 1 || op.asynchronous))
                || op.defaults.len() == op.args.len()
                || self
                    .operations
                    .values()
                    .any(|old| old.receiver_method.as_ref() == op.receiver_method.as_ref()))
        {
            return Err("invalid or duplicate host receiver method".into());
        }
        self.operations.insert(op.name.clone(), op);
        Ok(())
    }
    pub fn operation(&self, name: &str) -> Result<&HostOp> {
        self.operations
            .get(name)
            .ok_or_else(|| format!("unknown host operation: {name}"))
    }
    pub(crate) fn call(
        &self,
        name: &str,
        mut args: Vec<HostValue>,
        context: &HostContext,
    ) -> Result<(HostReply, HostType, bool, bool)> {
        let op = self.operation(name)?;
        // Closure invocation fills omitted parameter cells with unit. Defaults
        // cannot be unit, so only the optional trailing cells are removed here.
        while args.len() > op.args.len() - op.defaults.len()
            && matches!(args.last(), Some(HostValue::Unit))
        {
            args.pop();
        }
        if args.len() < op.args.len() - op.defaults.len()
            || args.len() > op.args.len()
            || !op.args.iter().zip(&args).all(|(t, v)| t.accepts(v))
        {
            return Err(format!("invalid arguments for host operation {name}"));
        }
        let omitted = op.args.len() - args.len();
        args.extend_from_slice(&op.defaults[op.defaults.len() - omitted..]);
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
    pub fn namespaces(&self) -> impl Iterator<Item = &HostOp> {
        self.operations.values().filter(|op| op.namespace.is_some())
    }
    pub fn methods(&self) -> impl Iterator<Item = &HostOp> {
        self.operations
            .values()
            .filter(|op| op.receiver_method.is_some() && !op.receiver_property)
    }
    pub fn properties(&self) -> impl Iterator<Item = &HostOp> {
        self.operations.values().filter(|op| op.receiver_property)
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
            let args = op.parameters_source(0);
            // Host failures are Result data or VM protocol faults, never language
            // Throw. A total declaration expresses that without a fake body.
            source.push_str(&format!(
                "export total fn {}({args}) {};\n",
                op.name,
                op.output_source()
            ));
        }
        for op in self.methods() {
            let (owner, method) = op.receiver_method.as_ref().expect("host method");
            // Receiver declarations are metadata, never executed. The compiler
            // dispatches their checked call sites directly to this host op.
            source.push_str(&format!(
                "fn (self {owner}) {method}{}({}) {} {{}}\n",
                if op.json_body { "<T>" } else { "" },
                op.parameters_source(1),
                if op.json_body {
                    "Promise<Result<T, string>>".into()
                } else {
                    op.output_source()
                }
            ));
        }
        source
    }
}
fn default_source(value: &HostValue) -> Option<String> {
    Some(match value {
        HostValue::Option(None) => "None".into(),
        HostValue::Option(Some(value)) => format!("Some({})", default_source(value)?),
        HostValue::Bool(value) => value.to_string(),
        HostValue::Number(value) if value.is_finite() => value.to_string(),
        HostValue::String(value) => serde_json::to_string(value).ok()?,
        HostValue::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(name, value)| Some(format!(
                    "{}: {}",
                    serde_json::to_string(name).ok()?,
                    default_source(value)?
                )))
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        ),
        HostValue::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(default_source)
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        ),
        _ => return None,
    })
}
