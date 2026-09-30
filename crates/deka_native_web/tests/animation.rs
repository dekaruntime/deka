use deka_native_web::NativePreview;
use serde_json::Value;
const SOURCE: &str = r#"export fn App() ReactNode {
    const [open, setOpen] = useState(0);
    return (<div>
        <button className="w-20 h-10" onClick={fn() {setOpen(1 - open);}}>Toggle</button>
        <button className={open == 1 ? "w-40 h-10 opacity-100 translate-x-20 bg-[#ffffff] transition-all duration-1000 ease-linear" : "w-20 h-10 opacity-0 translate-x-0 bg-[#000000] transition-all duration-1000 ease-linear"} onClick={fn() {setOpen(1 - open);}}>Panel</button>
    </div>);
}"#;
fn frame(p: &mut NativePreview, time: f64, reduced: bool) -> Value {
    serde_json::from_str(&p.frame_at(400., 240., 1., time, reduced)).unwrap()
}
fn panel(s: &Value) -> &Value {
    let nodes = s["nodes"].as_array().unwrap();
    let text = nodes.iter().find(|n| n["text"] == "Panel").unwrap();
    let parent = text["id"].as_str().unwrap().rsplit_once('/').unwrap().0;
    nodes.iter().find(|n| n["id"] == parent).unwrap()
}
fn near(a: &Value, b: f64) {
    assert!((a.as_f64().unwrap() - b).abs() < 0.01, "{a} != {b}");
}
#[test]
fn controlled_clock_interpolates_and_retargets_without_jumping() {
    let mut p = NativePreview::new();
    p.compile(SOURCE, false).unwrap();
    let initial = frame(&mut p, 0., false);
    assert_eq!(initial["targets"].as_array().unwrap().len(), 1);
    assert!(!p.pointer(10., 50.), "invisible panel cannot receive input");
    assert!(p.pointer(10., 10.));
    assert_eq!(frame(&mut p, 100., false)["animating"], true);
    let half = frame(&mut p, 600., false);
    near(&panel(&half)["rect"]["width"], 120.);
    near(&panel(&half)["rect"]["x"], 40.);
    let paint = half["paint"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["rect"] == panel(&half)["rect"])
        .unwrap();
    near(&paint["opacity"], 0.5);
    assert_eq!(paint["color"], 0x808080);
    assert!(!p.pointer(10., 50.), "old translated position must not hit");
    assert!(
        p.pointer(50., 50.),
        "moving target follows its painted position"
    );
    let reversed = frame(&mut p, 600., false);
    near(&panel(&reversed)["rect"]["x"], 40.);
    near(&panel(&frame(&mut p, 1100., false))["rect"]["x"], 20.);
    let end = frame(&mut p, 1600., false);
    assert_eq!(end["animating"], false);
    assert_eq!(end["targets"].as_array().unwrap().len(), 1);
}
#[test]
fn reduced_motion_reset_and_invalid_source_preserve_lifecycle() {
    let mut p = NativePreview::new();
    p.compile(SOURCE, false).unwrap();
    frame(&mut p, 0., false);
    p.pointer(10., 10.);
    frame(&mut p, 10., false);
    let reduced = frame(&mut p, 20., true);
    near(&panel(&reduced)["rect"]["x"], 80.);
    assert_eq!(reduced["animating"], false);
    assert!(p.compile("not valid source !!!", false).is_err());
    assert_eq!(frame(&mut p, 30., false), reduced);
    p.compile(SOURCE, true).unwrap();
    let reset = frame(&mut p, 40., false);
    near(&panel(&reset)["rect"]["x"], 0.);
    assert_eq!(reset["animating"], false);
}
