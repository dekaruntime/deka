use deka_wasm_guest::deka_export_json;

fn greet(args: Vec<serde_json::Value>) -> serde_json::Value {
    let name = args
        .first()
        .and_then(|value| value.as_str())
        .unwrap_or("world");
    serde_json::Value::String(format!("Hello, {}!", name))
}

deka_export_json!(greet);
