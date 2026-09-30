//! Emit scenes from the native build for comparison with real WASM in CI.
use deka_native_web::NativePreview;
use serde_json::{Value, json};
fn main() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../examples/native/layout-cases.json")).unwrap();
    let mut scenes = vec![];
    for case in cases {
        let mut preview = NativePreview::new();
        preview
            .compile(case["source"].as_str().unwrap(), false)
            .unwrap();
        for scale in [1., 2.] {
            let scene: Value = serde_json::from_str(&preview.frame(
                case["width"].as_f64().unwrap() as f32,
                case["height"].as_f64().unwrap() as f32,
                scale,
            ))
            .unwrap();
            scenes.push(json!({"name": case["name"], "scale": scale, "scene": scene}));
        }
    }
    println!("{}", serde_json::to_string(&scenes).unwrap());
}
