//! Typed JSON conversion shared by globals, receiver methods, and the JSON module.
//! The compiler supplies a concrete schema; JSON never introduces a dynamic value.
use crate::Result;
use crate::heap::{Handle, Heap, Record, Value};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value as Json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JsonShape {
    Leaf(String),
    Struct {
        name: String,
        identity: String,
        fields: Vec<JsonField>,
        embeds: Vec<String>,
    },
    Record(Vec<JsonField>),
    Tuple(Vec<JsonShape>),
    Array(Box<JsonShape>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonField {
    pub name: String,
    pub shape: JsonShape,
}

pub(crate) fn stringify(heap: &Heap, value: Handle, shape: &JsonShape) -> Result<String> {
    serde_json::to_string(&encode(heap, value, shape)?).map_err(|e| e.to_string())
}

fn encode(heap: &Heap, value: Handle, shape: &JsonShape) -> Result<Json> {
    match (shape, heap.get(value)?) {
        (JsonShape::Leaf(kind), Value::Number(n)) if kind == "number" => {
            if !n.is_finite() {
                return Err("JSON cannot encode a non-finite number".into());
            }
            // Preserve the established corpus format: whole doubles encode as 1, not 1.0.
            let number = if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n < i64::MAX as f64 {
                Number::from(*n as i64)
            } else {
                Number::from_f64(*n).ok_or("JSON cannot encode a non-finite number")?
            };
            Ok(Json::Number(number))
        }
        (JsonShape::Leaf(kind), Value::String(s)) if kind == "string" => {
            Ok(Json::String(s.clone()))
        }
        (JsonShape::Leaf(kind), Value::Bool(b)) if kind == "boolean" => Ok(Json::Bool(*b)),
        (JsonShape::Array(elem), Value::List(items)) => items
            .iter()
            .map(|item| encode(heap, *item, elem))
            .collect::<Result<Vec<_>>>()
            .map(Json::Array),
        (JsonShape::Tuple(elements), Value::List(items)) if items.len() == elements.len() => items
            .iter()
            .zip(elements)
            .map(|(item, elem)| encode(heap, *item, elem))
            .collect::<Result<Vec<_>>>()
            .map(Json::Array),
        (JsonShape::Record(fields), Value::Record(record)) => {
            let mut object = Map::new();
            for field in fields {
                let value = *record
                    .get(&field.name)
                    .ok_or_else(|| format!("JSON field `{}` is missing", field.name))?;
                object.insert(field.name.clone(), encode(heap, value, &field.shape)?);
            }
            Ok(Json::Object(object))
        }
        (
            JsonShape::Struct {
                name,
                identity,
                fields,
                ..
            },
            Value::Record(record),
        ) if record.struct_identity.as_ref() == Some(identity) => {
            let mut object = Map::new();
            for field in fields {
                let value = *record
                    .get(&field.name)
                    .ok_or_else(|| format!("JSON field `{}` is missing", field.name))?;
                object.insert(field.name.clone(), encode(heap, value, &field.shape)?);
            }
            // Nominal struct wrappers are the existing corpus wire format.
            Ok(Json::Object(
                [(name.clone(), Json::Object(object))].into_iter().collect(),
            ))
        }
        _ => Err("value does not match its checked JSON shape".into()),
    }
}

pub(crate) fn parse(
    heap: &mut Heap,
    text: &str,
    shape: &JsonShape,
    factories: Handle,
) -> Result<Handle> {
    let json: Json = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    decode(heap, &json, shape, factories, "$")
}

fn decode(
    heap: &mut Heap,
    json: &Json,
    shape: &JsonShape,
    factories: Handle,
    path: &str,
) -> Result<Handle> {
    let mismatch = || format!("{path}: JSON value does not match the expected type");
    match (shape, json) {
        (JsonShape::Leaf(kind), Json::Number(n)) if kind == "number" => {
            let n = n.as_f64().filter(|n| n.is_finite()).ok_or_else(mismatch)?;
            Ok(heap.alloc(Value::Number(n)))
        }
        (JsonShape::Leaf(kind), Json::String(s)) if kind == "string" => {
            Ok(heap.alloc(Value::String(s.clone())))
        }
        (JsonShape::Leaf(kind), Json::Bool(b)) if kind == "boolean" => {
            Ok(heap.alloc(Value::Bool(*b)))
        }
        (JsonShape::Array(elem), Json::Array(items)) => {
            let values = items
                .iter()
                .enumerate()
                .map(|(i, item)| decode(heap, item, elem, factories, &format!("{path}[{i}]")))
                .collect::<Result<Vec<_>>>()?;
            Ok(heap.alloc(Value::List(values)))
        }
        (JsonShape::Tuple(elements), Json::Array(items)) if items.len() == elements.len() => {
            let values = items
                .iter()
                .zip(elements)
                .enumerate()
                .map(|(i, (item, elem))| {
                    decode(heap, item, elem, factories, &format!("{path}[{i}]"))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(heap.alloc(Value::List(values)))
        }
        (JsonShape::Record(fields), Json::Object(object)) => {
            let mut record = Record::default();
            for field in fields {
                let field_path = format!("{path}.{}", field.name);
                let json = object
                    .get(&field.name)
                    .ok_or_else(|| format!("{field_path}: required JSON field is missing"))?;
                let value = decode(heap, json, &field.shape, factories, &field_path)?;
                record.order.push(field.name.clone());
                record.insert(field.name.clone(), value);
            }
            Ok(heap.alloc(Value::Record(record)))
        }
        (
            JsonShape::Struct {
                name,
                identity,
                fields,
                embeds,
            },
            Json::Object(wrapper),
        ) => {
            let object = wrapper
                .get(name)
                .and_then(Json::as_object)
                .ok_or_else(mismatch)?;
            let mut record = Record {
                struct_name: Some(name.clone()),
                struct_identity: Some(identity.clone()),
                embeds: embeds.clone(),
                ..Default::default()
            };
            for field in fields {
                let field_path = format!("{path}.{name}.{}", field.name);
                let json = object
                    .get(&field.name)
                    .ok_or_else(|| format!("{field_path}: required JSON field is missing"))?;
                let value = decode(heap, json, &field.shape, factories, &field_path)?;
                record.order.push(field.name.clone());
                record.insert(field.name.clone(), value);
            }
            let Value::Record(table) = heap.get(factories)? else {
                return Err("invalid JSON struct method table".into());
            };
            if let Some(methods) = table.get(identity) {
                let Value::Record(methods) = heap.get(*methods)? else {
                    return Err("invalid JSON struct methods".into());
                };
                record.fields.extend(methods.fields.clone());
            }
            Ok(heap.alloc(Value::Record(record)))
        }
        _ => Err(mismatch()),
    }
}
