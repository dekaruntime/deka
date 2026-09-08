//! Host execution and materialization for compiler-planned `build {}` values.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use engine::{RuntimeEngine, config as runtime_config};
use pool::{ExecutionMode, HandlerKey, PoolConfig, RequestData};
use runtime_core::framework::compiler_cache_dir;

use crate::env::init_env;
use crate::extensions::extensions_for_mode;

/// One compiler-planned build entry. The CLI owns plan parsing; runtime owns
/// execution, descriptor validation, and the virtual-module representation.
#[derive(Debug)]
pub struct BuildEntry {
    pub id: String,
    pub binding: String,
    pub entry: PathBuf,
    pub descriptor: serde_json::Value,
}

/// Execute every generated build entry and atomically replace the project's
/// virtual build-value modules only when every entry validates successfully.
/// Returns the validated `Ok(value)` JSON per slot id (input to the build
/// manifest's staticParams expansion).
pub fn materialize_build_values(
    project_root: &Path,
    entries: Vec<BuildEntry>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("build runtime: {err}"))?;
    rt.block_on(materialize_build_values_async(project_root, entries))
}

async fn materialize_build_values_async(
    project_root: &Path,
    entries: Vec<BuildEntry>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let cache_dir = compiler_cache_dir(project_root);
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let staging = tempfile::Builder::new()
        .prefix("build-values-")
        .tempdir_in(&cache_dir)
        .map_err(|err| format!("failed to create build-value staging directory: {err}"))?;

    if entries.is_empty() {
        replace_build_value_dir(&cache_dir, staging.path())?;
        return Ok(BTreeMap::new());
    }

    init_env();
    unsafe {
        std::env::set_var("DEKA_SECURITY_NO_PROMPT", "1");
    }
    let mut pool_config = PoolConfig::default();
    pool_config.num_workers = 1;
    pool_config.request_timeout_ms = 30_000;
    let runtime_cfg = runtime_config::RuntimeConfig::load();
    let serve_mode = runtime_config::ServeMode::Php;
    let extensions_provider = Arc::new(move || extensions_for_mode(&serve_mode));
    let engine = RuntimeEngine::new(
        pool_config.clone(),
        pool_config,
        &runtime_cfg,
        extensions_provider,
    );
    let module_root = project_root.to_string_lossy().into_owned();
    let mut values: BTreeMap<String, serde_json::Value> = BTreeMap::new();

    for entry in entries {
        validate_slot_id(&entry.id)?;
        let handler_entry = entry.entry.to_string_lossy().into_owned();
        let response = engine
            .execute(
                HandlerKey::new(format!("build:{}", entry.id)),
                RequestData {
                    handler_code: String::new(),
                    handler_entry: Some(handler_entry),
                    module_root: Some(module_root.clone()),
                    request_value: serde_json::Value::Null,
                    request_parts: None,
                    mode: ExecutionMode::Build,
                },
            )
            .await
            .map_err(|err| format!("build `{}`: {err}", entry.binding))?;
        if !response.success {
            return Err(format!(
                "build `{}`: {}",
                entry.binding,
                response
                    .error
                    .unwrap_or_else(|| "unknown execution error".to_string())
            ));
        }
        let encoded = response
            .result
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| format!("build `{}` returned no JSON result", entry.binding))?;
        let result: serde_json::Value = serde_json::from_str(&encoded)
            .map_err(|err| format!("build `{}` returned invalid JSON: {err}", entry.binding))?;
        let value = unwrap_result(&result, &entry.binding)?;
        validate_value(&entry.descriptor, value, "value")
            .map_err(|err| format!("build `{}` returned {err}", entry.binding))?;
        values.insert(entry.id.clone(), value.clone());
        let module = build_value_module(&entry.descriptor, value)?;
        std::fs::write(staging.path().join(format!("{}.js", entry.id)), module)
            .map_err(|err| format!("failed to materialize build `{}`: {err}", entry.binding))?;
    }

    replace_build_value_dir(&cache_dir, staging.path())?;
    Ok(values)
}

fn replace_build_value_dir(cache_dir: &Path, staged: &Path) -> Result<(), String> {
    let destination = cache_dir.join("build-values");
    if destination.exists() {
        std::fs::remove_dir_all(&destination)
            .map_err(|err| format!("failed to remove {}: {err}", destination.display()))?;
    }
    std::fs::rename(staged, &destination).map_err(|err| {
        format!(
            "failed to promote build values {} -> {}: {err}",
            staged.display(),
            destination.display()
        )
    })
}

fn validate_slot_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!("dsc emitted invalid build slot identifier `{id}`"));
    }
    Ok(())
}

fn unwrap_result<'a>(
    result: &'a serde_json::Value,
    binding: &str,
) -> Result<&'a serde_json::Value, String> {
    let object = result
        .as_object()
        .ok_or_else(|| format!("build `{binding}` must return Result<T, string>"))?;
    if object.get("__enum").and_then(serde_json::Value::as_str) != Some("Result") {
        return Err(format!("build `{binding}` must return Result<T, string>"));
    }
    match object.get("__case").and_then(serde_json::Value::as_str) {
        Some("Ok") => object
            .get("value")
            .ok_or_else(|| format!("build `{binding}` returned malformed Result.Ok")),
        Some("Err") => {
            let message = object
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("build entry returned Result.Err");
            Err(format!("build `{binding}` failed: {message}"))
        }
        _ => Err(format!("build `{binding}` returned malformed Result")),
    }
}

fn descriptor_node<'a>(descriptor: &'a serde_json::Value) -> Result<&'a str, String> {
    descriptor
        .get("node")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "an invalid compiler descriptor".to_string())
}

fn descriptor_field<'a>(
    descriptor: &'a serde_json::Value,
    key: &str,
) -> Result<&'a serde_json::Value, String> {
    descriptor
        .get(key)
        .ok_or_else(|| format!("an invalid compiler descriptor missing `{key}`"))
}

fn validate_value(
    descriptor: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    match descriptor_node(descriptor)? {
        "leaf" => match descriptor_field(descriptor, "kind")?.as_str() {
            Some("number") if value.is_number() => Ok(()),
            Some("string") if value.is_string() => Ok(()),
            Some("boolean") if value.is_boolean() => Ok(()),
            Some("none") if value.is_null() => Ok(()),
            Some("bytes") => validate_bytes(value, path),
            Some(kind) => Err(format!("{path} is not a {kind}")),
            None => Err("an invalid leaf compiler descriptor".to_string()),
        },
        "array" => {
            let values = value
                .as_array()
                .ok_or_else(|| format!("{path} is not an array"))?;
            let element = descriptor_field(descriptor, "elem")?;
            for (index, item) in values.iter().enumerate() {
                validate_value(element, item, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        "struct" => validate_struct(descriptor, value, path),
        "interface" => value
            .as_object()
            .map(|_| ())
            .ok_or_else(|| format!("{path} is not an object")),
        "newtype" => validate_value(descriptor_field(descriptor, "repr")?, value, path),
        "enum" => validate_enum(descriptor, value, path),
        "option" => validate_option(descriptor, value, path),
        "union" => {
            let members = descriptor_field(descriptor, "members")?
                .as_array()
                .ok_or_else(|| "an invalid union compiler descriptor".to_string())?;
            if members
                .iter()
                .any(|member| validate_value(member, value, path).is_ok())
            {
                Ok(())
            } else {
                Err(format!("{path} does not match any union member"))
            }
        }
        "recurse" => Err("a recursive build value descriptor".to_string()),
        node => Err(format!("an unsupported compiler descriptor node `{node}`")),
    }
}

fn validate_bytes(value: &serde_json::Value, path: &str) -> Result<(), String> {
    let bytes = value
        .get("__deka_bytes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{path} is not bytes"))?;
    if bytes
        .iter()
        .all(|item| item.as_u64().is_some_and(|byte| byte <= 255))
    {
        Ok(())
    } else {
        Err(format!("{path} contains an invalid byte"))
    }
}

fn validate_struct(
    descriptor: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{path} is not a struct object"))?;
    let fields = descriptor_field(descriptor, "fields")?
        .as_array()
        .ok_or_else(|| "an invalid struct compiler descriptor".to_string())?;
    for field in fields {
        let name = descriptor_field(field, "name")?
            .as_str()
            .ok_or_else(|| "an invalid struct field descriptor".to_string())?;
        let optional = descriptor_field(field, "optional")?
            .as_bool()
            .ok_or_else(|| "an invalid struct field descriptor".to_string())?;
        match object.get(name) {
            Some(field_value) => validate_value(
                descriptor_field(field, "ty")?,
                field_value,
                &format!("{path}.{name}"),
            )?,
            None if optional => {}
            None => return Err(format!("{path}.{name} is missing")),
        }
    }
    Ok(())
}

fn validate_enum(
    descriptor: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{path} is not an enum value"))?;
    let name = descriptor_field(descriptor, "name")?
        .as_str()
        .ok_or_else(|| "an invalid enum compiler descriptor".to_string())?;
    if object.get("__enum").and_then(serde_json::Value::as_str) != Some(name) {
        return Err(format!("{path} is not a {name}"));
    }
    let case = object
        .get("__case")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("{path} has no enum case"))?;
    let cases = descriptor_field(descriptor, "cases")?
        .as_array()
        .ok_or_else(|| "an invalid enum compiler descriptor".to_string())?;
    let selected = cases.iter().find(|candidate| {
        candidate
            .as_array()
            .and_then(|items| items.first())
            .and_then(serde_json::Value::as_str)
            == Some(case)
    });
    let payload = selected
        .and_then(|candidate| candidate.as_array())
        .and_then(|items| items.get(1))
        .ok_or_else(|| format!("{path} has unknown {name} case `{case}`"))?;
    if payload.is_null() {
        Ok(())
    } else {
        let value = object
            .get("value")
            .ok_or_else(|| format!("{path} {name}.{case} payload is missing"))?;
        validate_value(payload, value, &format!("{path}.value"))
    }
}

fn validate_option(
    descriptor: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{path} is not an Option"))?;
    if object.get("__enum").and_then(serde_json::Value::as_str) != Some("Option") {
        return Err(format!("{path} is not an Option"));
    }
    match object.get("__case").and_then(serde_json::Value::as_str) {
        Some("None") => Ok(()),
        Some("Some") => validate_value(
            descriptor_field(descriptor, "inner")?,
            object
                .get("value")
                .ok_or_else(|| format!("{path}.value is missing"))?,
            &format!("{path}.value"),
        ),
        _ => Err(format!("{path} has an invalid Option case")),
    }
}

fn build_value_module(
    descriptor: &serde_json::Value,
    value: &serde_json::Value,
) -> Result<String, String> {
    let descriptor = serde_json::to_string(descriptor)
        .map_err(|err| format!("failed to serialize build descriptor: {err}"))?;
    let value = serde_json::to_string(value)
        .map_err(|err| format!("failed to serialize build value: {err}"))?;
    Ok(format!(
        "const __dekaBuildHydrate = (factories, descriptor, value, strict) => {{\n\
  switch (descriptor.node) {{\n\
    case \"leaf\":\n\
      return descriptor.kind === \"bytes\" ? new Uint8Array(value.__deka_bytes) : value;\n\
    case \"array\": return value.map((item) => __dekaBuildHydrate(factories, descriptor.elem, item, strict));\n\
    case \"struct\": {{\n\
      const fields = {{}};\n\
      for (const field of descriptor.fields) if (Object.hasOwn(value, field.name)) fields[field.name] = __dekaBuildHydrate(factories, field.ty, value[field.name], strict);\n\
      const factory = factories[descriptor.name];\n\
      if (typeof factory === \"function\") return factory(fields);\n\
      if (strict) throw new Error(`build hydration is missing the declared ${{descriptor.name}} factory`);\n\
      const proto = Object.create(null);\n\
      Object.defineProperty(proto, \"__deka_struct\", {{ value: descriptor.name, enumerable: false }});\n\
      const out = Object.create(proto);\n\
      Object.assign(out, fields);\n\
      return out;\n\
    }}\n\
    case \"interface\": return value;\n\
    case \"newtype\": {{\n\
      const payload = __dekaBuildHydrate(factories, descriptor.repr, value, strict);\n\
      const factory = factories[descriptor.name];\n\
      if (typeof factory === \"function\") return factory(payload);\n\
      if (strict) throw new Error(`build hydration is missing the declared ${{descriptor.name}} factory`);\n\
      const proto = Object.create(null);\n\
      Object.defineProperty(proto, \"__deka_newtype\", {{ value: descriptor.name, enumerable: false }});\n\
      Object.defineProperty(proto, Symbol.for(\"deka.nt\"), {{ value: payload, enumerable: false }});\n\
      proto.toJSON = function () {{ return this[Symbol.for(\"deka.nt\")]; }};\n\
      return Object.create(proto);\n\
    }}\n\
    case \"enum\": {{\n\
      const item = descriptor.cases.find(([name]) => name === value.__case);\n\
      const factory = factories[descriptor.name];\n\
      if (factory) return item[1] === null\n\
        ? factory[value.__case]\n\
        : factory[value.__case](__dekaBuildHydrate(factories, item[1], value.value, strict));\n\
      if (strict) throw new Error(`build hydration is missing the declared ${{descriptor.name}} factory`);\n\
      return item[1] === null\n\
        ? {{ __enum: descriptor.name, __case: value.__case, name: value.__case }}\n\
        : {{ __enum: descriptor.name, __case: value.__case, name: value.__case, value: __dekaBuildHydrate(factories, item[1], value.value, strict) }};\n\
    }}\n\
    case \"option\": return value.__case === \"None\"\n\
      ? {{ __enum: \"Option\", __case: \"None\", name: \"None\" }}\n\
      : {{ __enum: \"Option\", __case: \"Some\", name: \"Some\", value: __dekaBuildHydrate(factories, descriptor.inner, value.value, strict) }};\n\
    case \"union\": {{\n\
      for (const member of descriptor.members) {{\n\
        try {{ return __dekaBuildHydrate(factories, member, value, strict); }} catch (_err) {{}}\n\
      }}\n\
      throw new Error(\"build value does not match a union member\");\n\
    }}\n\
    default: throw new Error(`unsupported build descriptor ${{descriptor.node}}`);\n\
  }}\n\
}};\n\
const __dekaBuildDescriptor = {descriptor};\n\
const __dekaBuildValue = {value};\n\
export const hydrate = (factories = {{}}) => __dekaBuildHydrate(factories, __dekaBuildDescriptor, __dekaBuildValue, true);\n\
export const value = __dekaBuildHydrate({{}}, __dekaBuildDescriptor, __dekaBuildValue, false);\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_structured_values_against_compiler_descriptors() {
        let descriptor = serde_json::json!({
            "node": "struct",
            "name": "User",
            "fields": [
                { "name": "name", "optional": false, "ty": { "node": "leaf", "kind": "string", "name": "string" } },
                { "name": "active", "optional": true, "ty": { "node": "leaf", "kind": "boolean", "name": "boolean" } }
            ]
        });
        assert!(
            validate_value(&descriptor, &serde_json::json!({ "name": "Ada" }), "value").is_ok()
        );
        assert_eq!(
            validate_value(&descriptor, &serde_json::json!({ "name": 1 }), "value").unwrap_err(),
            "value.name is not a string"
        );
    }

    #[test]
    fn rejects_raw_values_and_surfaces_result_errors() {
        assert!(unwrap_result(&serde_json::json!(["Ada"]), "labels").is_err());
        assert_eq!(
            unwrap_result(
                &serde_json::json!({ "__enum": "Result", "__case": "Err", "error": "missing file" }),
                "labels"
            )
            .unwrap_err(),
            "build `labels` failed: missing file"
        );
    }

    #[test]
    fn hydrate_requires_declared_factories_but_legacy_value_stays_available() {
        let module = build_value_module(
            &serde_json::json!({
                "node": "struct",
                "name": "User",
                "fields": []
            }),
            &serde_json::json!({}),
        )
        .expect("virtual module serializes");
        assert!(
            module.contains("build hydration is missing the declared ${descriptor.name} factory"),
            "{module}"
        );
        assert!(
            module.contains("export const hydrate = (factories = {}) => __dekaBuildHydrate(factories, __dekaBuildDescriptor, __dekaBuildValue, true);"),
            "{module}"
        );
        assert!(
            module.contains("export const value = __dekaBuildHydrate({}, __dekaBuildDescriptor, __dekaBuildValue, false);"),
            "{module}"
        );
    }
}
