use deka_native_web::NativePreview;
use serde_json::{Value, json};

fn source(body: &str) -> String {
    format!("export fn App() ReactNode {{ return ({body}); }}")
}
fn render(body: &str, width: f32, height: f32) -> Value {
    let mut preview = NativePreview::new();
    preview.compile(&source(body), false).unwrap();
    serde_json::from_str(&preview.frame(width, height, 1.)).unwrap()
}
fn node<'a>(scene: &'a Value, id: &str) -> &'a Value {
    scene["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("missing {id}: {scene}"))
}
fn bounds(scene: &Value, id: &str, x: f64, y: f64, w: f64, h: f64) {
    let rect = &node(scene, id)["rect"];
    for (key, expected) in [("x", x), ("y", y), ("width", w), ("height", h)] {
        let actual = rect[key].as_f64().unwrap();
        assert!(
            (actual - expected).abs() < 0.02,
            "{id}.{key}: {actual} != {expected}"
        );
    }
}
#[test]
fn centers_boxes_and_resolves_edges_and_percentage_sizes() {
    let s = render(
        r#"<div className="w-full h-full items-center justify-center"><div className="w-20 h-10 bg-[#663399]" /></div>"#,
        320.,
        200.,
    );
    bounds(&s, "root", 0., 0., 320., 200.);
    bounds(&s, "root/0", 120., 80., 80., 40.);
    let s = render(
        r#"<div className="w-full p-4 pl-2 gap-2"><div className="w-1/2 min-w-20 max-w-24 h-10 ml-3"/><div className="w-full h-5"/></div>"#,
        320.,
        200.,
    );
    bounds(&s, "root/0", 20., 16., 96., 40.);
    bounds(&s, "root/1", 8., 64., 296., 20.);
}
#[test]
fn flex_growth_shrinkage_and_wrap_are_observable() {
    let s = render(
        r#"<div className="flex-row w-60 h-20 gap-2 items-center"><div className="w-10 h-10 flex-none"/><div className="grow h-5"/><div className="w-10 h-10 flex-none"/></div>"#,
        320.,
        200.,
    );
    bounds(&s, "root/1", 48., 30., 144., 20.);
    let s = render(
        r#"<div className="flex-row w-40"><div className="w-30 h-10 min-w-0"/><div className="w-30 h-10 min-w-0"/></div>"#,
        320.,
        200.,
    );
    bounds(&s, "root/0", 0., 0., 80., 40.);
    bounds(&s, "root/1", 80., 0., 80., 40.);
    let s = render(
        r#"<div className="flex-row flex-wrap w-40 gap-x-2 gap-y-3"><div className="w-24 h-10 flex-none"/><div className="w-24 h-10 flex-none"/></div>"#,
        320.,
        200.,
    );
    bounds(&s, "root/1", 0., 52., 96., 40.);
}
#[test]
fn overflow_clips_paint_and_hit_testing_but_not_layout() {
    let body = r#"export fn App() ReactNode { const [n, setN] = useState(0); return (<div className="w-20 h-10 overflow-hidden"><button className="w-40 h-20 flex-none" onClick={fn(){setN(n+1);}}>Count: {string(n)}</button></div>); }"#;
    let mut p = NativePreview::new();
    p.compile(body, false).unwrap();
    let s: Value = serde_json::from_str(&p.frame(320., 200., 1.)).unwrap();
    bounds(&s, "root/0", 0., 0., 160., 80.);
    assert_eq!(
        s["targets"][0]["clip"],
        json!({"x":0.,"y":0.,"width":80.,"height":40.})
    );
    assert!(!p.pointer(100., 20.));
    assert!(p.pointer(20., 20.));
    let s: Value = serde_json::from_str(&p.frame(320., 200., 1.)).unwrap();
    assert_eq!(node(&s, "root/0/0")["text"], "Count: 1");
    assert!(
        s["paint"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["image"].is_string())
            .all(|p| p["clip"]["width"] == 80.)
    );
}
#[test]
fn text_interpolation_is_one_run_and_wrap_matches_measurement() {
    let mut p = NativePreview::new();
    p.compile(r#"export fn App() ReactNode { const [n, setN] = useState(0); return (<div className="w-20"><span>Hello {string(n)} wonderful world</span></div>); }"#, false).unwrap();
    let narrow: Value = serde_json::from_str(&p.frame(320., 200., 1.)).unwrap();
    let run = node(&narrow, "root/0/0");
    assert_eq!(run["text"], "Hello 0 wonderful world");
    let height = run["rect"]["height"].as_f64().unwrap();
    assert!(height > 32., "text must wrap: {height}");
    // DPI changes glyph resolution, never logical layout.
    let retina: Value = serde_json::from_str(&p.frame(320., 200., 2.)).unwrap();
    assert_eq!(narrow["nodes"], retina["nodes"]);
    let nowrap = render(
        r#"<div className="w-20 whitespace-nowrap"><span>Hello 0 wonderful world</span></div>"#,
        320.,
        200.,
    );
    assert!(
        node(&nowrap, "root/0/0")["rect"]["height"]
            .as_f64()
            .unwrap()
            < height
    );
}
