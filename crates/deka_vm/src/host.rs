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
    /// Clone a typed reference to the same host allocation without a proxy.
    #[cfg(feature = "ui")]
    pub(crate) fn shared<T: Any>(&self) -> Option<Rc<T>> {
        self.resource.clone().downcast::<T>().ok()
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
/// A named enum schema shared by checking, marshalling and error channels.
#[derive(Clone, Debug, PartialEq)]
pub struct HostEnum {
    pub name: String,
    pub cases: Vec<(String, Option<HostType>)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HostStruct {
    pub name: String,
    pub fields: BTreeMap<String, HostType>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HostEnumError {
    pub schema: HostEnum,
    pub case: String,
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
    Enum {
        name: String,
        case: String,
        payload: Option<Box<HostValue>>,
    },
    Struct {
        name: String,
        fields: BTreeMap<String, HostValue>,
    },
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
    Enum(Box<HostEnum>),
    Struct(Box<HostStruct>),
    /// A synchronous or asynchronous language callback with a single checked
    /// argument/return contract. Result failures are data, not thrown values.
    TypedCallback {
        args: Vec<HostType>,
        result: Box<HostType>,
        result_channel: bool,
    },
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
            | (Self::Callback, HostValue::Callback(_))
            | (Self::TypedCallback { .. }, HostValue::Callback(_)) => true,
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
            (
                Self::Enum(schema),
                HostValue::Enum {
                    name,
                    case,
                    payload,
                },
            ) => {
                name == &schema.brand()
                    && schema.cases.iter().any(|(label, ty)| {
                        label == case
                            && match (ty, payload) {
                                (None, None) => true,
                                (Some(ty), Some(value)) => ty.accepts(value),
                                _ => false,
                            }
                    })
            }
            (Self::Struct(schema), HostValue::Struct { name, fields }) => {
                name == &schema.brand()
                    && Self::Record(schema.fields.clone())
                        .accepts(&HostValue::Record(fields.clone()))
            }
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
            Self::TypedCallback {
                args,
                result,
                result_channel,
            } => {
                let parameters = args
                    .iter()
                    .map(HostType::source)
                    .collect::<Vec<_>>()
                    .join(", ");
                let output = if *result_channel {
                    format!("Result<{}, string>", result.source())
                } else {
                    result.source()
                };
                format!("(fn({parameters}) {output}) | (fn({parameters}) Promise<{output}>)")
            }
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
            Self::Enum(schema) => schema.brand(),
            Self::Struct(schema) => schema.brand(),
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
            Self::TypedCallback { args, result, .. } => {
                for ty in args.iter().chain(std::iter::once(result.as_ref())) {
                    ty.validate(handles, depth + 1)?;
                }
            }
            Self::Enum(schema) => {
                validate_nominal_name(&schema.name)?;
                if schema.cases.is_empty() {
                    return Err("host enum needs a case".into());
                }
                let mut names = BTreeSet::new();
                for (case, payload) in &schema.cases {
                    if !identifier(case) || !names.insert(case) {
                        return Err("invalid or duplicate host enum case".into());
                    }
                    if let Some(ty) = payload {
                        ty.validate(handles, depth + 1)?;
                    }
                }
            }
            Self::Struct(schema) => {
                validate_nominal_name(&schema.name)?;
                Self::Record(schema.fields.clone()).validate(handles, depth + 1)?;
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
            (
                Self::TypedCallback {
                    args,
                    result,
                    result_channel,
                },
                HostValue::Callback(callback),
            ) => HostValue::Callback(callback.with_signature(
                args.clone(),
                *result.clone(),
                *result_channel,
            )),
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
            (
                Self::Enum(schema),
                HostValue::Enum {
                    name,
                    case,
                    payload,
                },
            ) => {
                let ty = &schema
                    .cases
                    .iter()
                    .find(|(label, _)| label == &case)
                    .expect("validated host enum")
                    .1;
                HostValue::Enum {
                    name,
                    case,
                    payload: payload.map(|value| {
                        Box::new(
                            ty.as_ref()
                                .expect("validated enum payload")
                                .normalize(*value),
                        )
                    }),
                }
            }
            (Self::Struct(schema), HostValue::Struct { name, fields }) => {
                let HostValue::Record(fields) =
                    Self::Record(schema.fields.clone()).normalize(HostValue::Record(fields))
                else {
                    unreachable!()
                };
                HostValue::Struct { name, fields }
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
    pub result_error: Option<HostEnumError>,
    /// Synchronous operational failures raise a typed JsError.
    pub exception_channel: bool,
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
    /// Operational string failures become a declared string-payload enum case.
    pub fn with_enum_result_channel(mut self, schema: HostEnum, case: &str) -> Self {
        self.result_channel = true;
        self.result_error = Some(HostEnumError {
            schema,
            case: case.into(),
        });
        self
    }
    /// Declare synchronous `Exception<T, JsError>`; async channels fail closed.
    pub fn with_exception_channel(mut self) -> Self {
        self.exception_channel = true;
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
        let value = if self.exception_channel {
            format!("Exception<{}, JsError>", self.result.source())
        } else if self.result_channel {
            format!(
                "Result<{}, {}>",
                self.result.source(),
                self.result_error
                    .as_ref()
                    .map_or_else(|| "string".to_string(), |error| error.schema.brand())
            )
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
            result_error: None,
            exception_channel: false,
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
#[derive(Clone)]
pub struct Hosts {
    operations: BTreeMap<String, HostOp>,
    nominal: BTreeMap<String, HostType>,
}
impl Default for Hosts {
    fn default() -> Self {
        let hosts = Self {
            operations: BTreeMap::new(),
            nominal: BTreeMap::new(),
        };
        #[cfg(feature = "ui")]
        let mut hosts = hosts;
        #[cfg(feature = "ui")]
        hosts
            .bind_view_tree(Rc::new(std::cell::RefCell::new(
                crate::component::tree::Tree::default(),
            )))
            .expect("valid built-in view catalog");
        hosts
    }
}
impl Hosts {
    #[cfg(feature = "ui")]
    pub(crate) fn bind_view_tree(
        &mut self,
        tree: Rc<std::cell::RefCell<crate::component::tree::Tree>>,
    ) -> Result<()> {
        for op in crate::component::view_operations(tree) {
            // Rebind the canonical catalog to this app's session. Custom host
            // schemas cannot replace these already registered operation names.
            self.operations.remove(&op.name);
            self.register(op)?;
        }
        Ok(())
    }
    pub fn register(&mut self, op: HostOp) -> Result<()> {
        if op.exception_channel
            && (op.asynchronous
                || op.result_channel
                || op.result_error.is_some()
                || op.global_value.is_some()
                || op.receiver_property
                || op.json_body)
        {
            return Err("Exception host channel requires a synchronous throwing function".into());
        }
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
        if let Some(error) = &op.result_error {
            if !op.result_channel
                || !error
                    .schema
                    .cases
                    .iter()
                    .any(|(name, ty)| name == &error.case && ty.as_ref() == Some(&HostType::String))
            {
                return Err("host enum error needs a declared string-payload case".into());
            }
            HostType::Enum(Box::new(error.schema.clone())).validate(&mut BTreeSet::new(), 0)?;
        }
        let nominal = Self::nominal_schemas(self.operations.values().chain(Some(&op)))?;
        self.operations.insert(op.name.clone(), op);
        self.nominal = nominal;
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
    ) -> Result<(HostReply, HostType, bool, bool, Option<HostEnumError>)> {
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
        Ok((
            reply,
            op.result.clone(),
            op.asynchronous,
            op.result_channel,
            op.result_error.clone(),
        ))
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
        let mut source = self.nominal_source();
        source.push_str(
            &handles
                .into_iter()
                .map(|name| format!("export opaque type {name};\n"))
                .collect::<String>(),
        );
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
fn validate_nominal_name(name: &str) -> Result<()> {
    if !identifier(name)
        || !name.starts_with(char::is_uppercase)
        || matches!(
            name,
            "Option" | "Result" | "Promise" | "Exception" | "Array" | "Type"
        )
    {
        return Err("invalid named host type".into());
    }
    Ok(())
}
impl HostEnum {
    pub fn brand(&self) -> String {
        crate::native_brand::brand(&self.name)
    }
}
impl HostStruct {
    pub fn brand(&self) -> String {
        crate::native_brand::brand(&self.name)
    }
}
impl Hosts {
    fn nominal_schemas<'a>(
        operations: impl Iterator<Item = &'a HostOp>,
    ) -> Result<BTreeMap<String, HostType>> {
        fn collect(ty: &HostType, out: &mut BTreeMap<String, HostType>) -> Result<()> {
            let name = match ty {
                HostType::Enum(schema) => Some(&schema.name),
                HostType::Struct(schema) => Some(&schema.name),
                HostType::Handle(name) => Some(name),
                _ => None,
            };
            if let Some(name) = name {
                if let Some(old) = out.get(name) {
                    if old != ty {
                        return Err(format!("conflicting host type schema: {name}"));
                    }
                } else {
                    out.insert(name.clone(), ty.clone());
                }
            }
            match ty {
                HostType::Enum(schema) => {
                    for (_, payload) in &schema.cases {
                        if let Some(ty) = payload {
                            collect(ty, out)?;
                        }
                    }
                }
                HostType::Struct(schema) => {
                    for ty in schema.fields.values() {
                        collect(ty, out)?;
                    }
                }
                HostType::Record(fields) => {
                    for ty in fields.values() {
                        collect(ty, out)?;
                    }
                }
                HostType::List(ty) | HostType::Option(ty) => collect(ty, out)?,
                HostType::TypedCallback { args, result, .. } => {
                    for ty in args.iter().chain(std::iter::once(result.as_ref())) {
                        collect(ty, out)?;
                    }
                }
                HostType::Tuple(items) => {
                    for ty in items {
                        collect(ty, out)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut out = BTreeMap::new();
        for op in operations {
            for ty in op.args.iter().chain(std::iter::once(&op.result)) {
                collect(ty, &mut out)?;
            }
            if let Some(error) = &op.result_error {
                collect(&HostType::Enum(Box::new(error.schema.clone())), &mut out)?;
            }
        }
        Ok(out)
    }
    #[cfg(feature = "compiler")]
    pub(crate) fn nominal_types_for(
        &self,
        includes: impl Fn(&str) -> bool,
    ) -> BTreeMap<String, HostType> {
        Self::nominal_schemas(
            self.operations
                .iter()
                .filter_map(|(name, op)| includes(name).then_some(op)),
        )
        .expect("registered host schemas were validated together")
    }
    pub(crate) fn nominal_types(&self) -> BTreeMap<String, HostType> {
        self.nominal.clone()
    }
    pub(crate) fn nominal_type(&self, brand: &str) -> Option<&HostType> {
        self.nominal.get(public_native_name(brand))
    }
    fn nominal_source(&self) -> String {
        let mut out = String::new();
        for ty in self.nominal_types().values() {
            match ty {
                HostType::Struct(schema) => {
                    out.push_str(&format!("struct {} {{\n", schema.brand()));
                    for (name, ty) in &schema.fields {
                        out.push_str(&format!("{name}: {}\n", ty.source()));
                    }
                    out.push_str("}\n");
                    out.push_str(&format!("export {{ {} }};\n", schema.brand()));
                }
                HostType::Enum(schema) => {
                    out.push_str(&format!("enum {} {{\n", schema.brand()));
                    for (name, payload) in &schema.cases {
                        out.push_str(&format!(
                            "{name}{} ,\n",
                            payload
                                .as_ref()
                                .map(|ty| format!("({})", ty.source()))
                                .unwrap_or_default()
                        ));
                    }
                    out.push_str("}\n");
                    out.push_str(&format!("export {{ {} }};\n", schema.brand()));
                }
                _ => {}
            }
        }
        out
    }
}

pub(crate) use crate::native_brand::public_name as public_native_name;
