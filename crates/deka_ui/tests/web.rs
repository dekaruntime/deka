#![cfg(feature = "web")]
use deka_ui::{Event, UiApp, prelude::*, web::BrowserApp};
use serde_json::Value;
fn scene(app: &mut BrowserApp) -> Value {
    serde_json::from_str(&app.frame_at(400., 300., 2., 0., true).unwrap()).unwrap()
}
#[test]
fn browser_click_keyboard_and_resize_use_retained_bindings() {
    let mut app = BrowserApp::new(UiApp::new(|| {
        let mut count = signal(0);
        view! {<view><p>"Count: {count}"</p><button onClick={move |_|count+=1}>"Add"</button></view>}
    }));
    let first = scene(&mut app);
    let r = &first["targets"][0]["rect"];
    assert!(app.pointer(
        r["x"].as_f64().unwrap() as f32 + 2.,
        r["y"].as_f64().unwrap() as f32 + 2.
    ));
    assert!(
        scene(&mut app)["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["text"] == "Count: 1")
    );
    assert!(app.key("Enter", false));
    assert!(
        scene(&mut app)["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["text"] == "Count: 2")
    );
    let resized: Value =
        serde_json::from_str(&app.frame_at(200., 600., 1., 16., true).unwrap()).unwrap();
    assert_eq!(resized["width"], 200.);
    assert_eq!(resized["height"], 600.);
    app.blur();
    assert!(!app.key("Enter", false));
}
#[test]
fn input_dispatch_updates_the_same_bound_text_and_disposal_frees_signals() {
    let state = std::rc::Rc::new(std::cell::Cell::new(None));
    let captured = state.clone();
    let mut app = BrowserApp::new(UiApp::new(move || {
        let text = signal(String::new());
        captured.set(Some(text));
        view! {<view><input id="name" value={move ||text.get().unwrap()} onInput={move |event|if let Event::Input(value)=event {text.set(value);}}/><p>"Hello {text}"</p></view>}
    }));
    scene(&mut app);
    let inputs: Value = serde_json::from_str(&app.inputs()).unwrap();
    assert_eq!(inputs.as_array().unwrap().len(), 1);
    assert!(app.input(inputs[0]["id"].as_str().unwrap(), "Sami"));
    assert!(
        scene(&mut app)["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["text"] == "Hello Sami")
    );
    drop(app);
    assert!(state.get().unwrap().get().is_err());
}
