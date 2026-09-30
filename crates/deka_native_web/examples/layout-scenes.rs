//! Emit scenes from the native build for comparison with real WASM in CI.
use deka_native_web::NativePreview;
use serde_json::{Value, json};
fn main() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../examples/native/layout-cases.json")).unwrap();
    let mut scenes = vec![];
    for case in cases {
        for scale in [1., 2.] {
            let mut preview = NativePreview::new();
            preview
                .compile(case["source"].as_str().unwrap(), false)
                .unwrap();
            let default_steps = vec![json!({})];
            let steps = case["steps"].as_array().unwrap_or(&default_steps);
            let mut scene = Value::Null;
            for (index, step) in steps.iter().enumerate() {
                if step["click"] == true {
                    let r = &scene["targets"][0]["rect"];
                    assert!(preview.pointer(
                        (r["x"].as_f64().unwrap() + 4.) as f32,
                        (r["y"].as_f64().unwrap() + 4.) as f32
                    ));
                }
                let w = case["width"].as_f64().unwrap() as f32;
                let h = case["height"].as_f64().unwrap() as f32;
                let frame = if let Some(t) = step["time"].as_f64() {
                    preview.frame_at(w, h, scale, t, step["reduced"] == true)
                } else {
                    preview.frame(w, h, scale)
                };
                scene = serde_json::from_str(&frame).unwrap();
                scenes.push(
                    json!({"name": case["name"], "scale": scale, "step": index, "scene": scene}),
                );
            }
        }
    }
    println!("{}", serde_json::to_string(&scenes).unwrap());
}
