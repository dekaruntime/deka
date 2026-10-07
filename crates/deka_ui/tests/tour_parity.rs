//! Complete scene equality against the unchanged DekaScript tour/VM reference.
use deka_native_ir::Node;
use deka_native_ui::scene::{Renderer, Scene};
use deka_ui::UiApp;
use deka_vm::{Hosts, compiler, ui::UiSession};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

use deka_ui::tour::{self, Action, handler_nodes};
#[test]
fn every_original_lesson_has_a_runnable_rust_twin_and_a_gate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    fn inventory(root: &Path, extension: &str) -> BTreeSet<String> {
        std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == extension))
            .map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
            .collect()
    }
    let expected: BTreeSet<_> = tour::LESSONS
        .iter()
        .map(|lesson| lesson.id.to_owned())
        .collect();
    assert_eq!(expected.len(), 27);
    assert_eq!(
        inventory(&root.join("../deka_fmt/tests/fixtures/tour"), "dsx"),
        expected
    );
    assert_eq!(inventory(&root.join("examples/tour"), "rs"), expected);
    let manifest = include_str!("../Cargo.toml");
    for name in tour::LESSONS.iter().map(|lesson| lesson.id) {
        assert!(manifest.contains(&format!("name = \"tour-{name}\"")));
    }
}

fn serialized(scene: &Scene) -> Value {
    let mut value = serde_json::to_value(scene).unwrap();
    // The renderer's glyph-image cache is an unordered map. Preserve every
    // image/id/byte; canonicalize only that collection's iteration order.
    value["images"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(|image| image["id"].as_str().unwrap().to_owned());
    value
}
fn first_difference(left: &Value, right: &Value, path: &str) -> Option<String> {
    if left == right {
        return None;
    }
    match (left, right) {
        (Value::Object(left), Value::Object(right)) if left.keys().eq(right.keys()) => {
            left.iter().find_map(|(key, value)| {
                first_difference(value, &right[key], &format!("{path}.{key}"))
            })
        }
        (Value::Array(left), Value::Array(right)) if left.len() == right.len() => left
            .iter()
            .zip(right)
            .enumerate()
            .find_map(|(index, (a, b))| first_difference(a, b, &format!("{path}[{index}]"))),
        (Value::Array(left), Value::Array(right)) => Some(format!(
            "{path}: lengths VM={} Rust={}",
            left.len(),
            right.len()
        )),
        _ => Some(format!("{path}: VM={left} Rust={right}")),
    }
}
struct Pair {
    name: &'static str,
    vm: UiSession,
    rust: UiApp,
    vm_renderer: Renderer,
    rust_renderer: Renderer,
    viewport: (f32, f32, f32),
    reduced: bool,
    comparisons: usize,
}
impl Pair {
    fn frame(&mut self, time: f64) -> (Scene, Scene) {
        let (width, height, scale) = self.viewport;
        let vm =
            self.vm_renderer
                .render_at(self.vm.tree(), width, height, scale, time, self.reduced);
        let rust = self.rust_renderer.render_at(
            &self.rust.tree(),
            width,
            height,
            scale,
            time,
            self.reduced,
        );
        if let Some(difference) = first_difference(&serialized(&vm), &serialized(&rust), "scene") {
            panic!(
                "{} {:?} reduced={} time={time}ms: {difference}",
                self.name, self.viewport, self.reduced
            );
        }
        self.comparisons += 1;
        (vm, rust)
    }
    fn click(&mut self, handler: usize, time: f64) {
        let (scene, rust_scene) = self.frame(time);
        let point = tour::click_point(&scene, handler);
        if let Some((x, y)) = point {
            let vm_hit = scene.hit(x, y).unwrap();
            let rust_hit = rust_scene.hit(x, y).expect("Rust missed the same point");
            assert_eq!(
                (vm_hit.id.as_str(), vm_hit.handler),
                (rust_hit.id.as_str(), rust_hit.handler)
            );
        } else {
            // A clipped handler still needs event/effect coverage. Full-scene
            // parity above proves the clipping; dispatch the retained route.
            let vm = handler_nodes(self.vm.tree());
            let rust = handler_nodes(&self.rust.tree());
            let id = vm
                .iter()
                .find(|(_, token)| *token == handler)
                .unwrap_or_else(|| panic!("{}: missing retained handler {handler}", self.name));
            assert_eq!(rust.iter().find(|(_, token)| *token == handler), Some(id));
        }
        let vm_before = self.vm.tree().clone();
        let rust_before = self.rust.tree();
        self.vm.click(handler).unwrap();
        assert!(self.rust.dispatch(handler));
        self.assert_click_effect(&vm_before, self.vm.tree(), handler, "VM");
        self.assert_click_effect(&rust_before, &self.rust.tree(), handler, "Rust");
        self.frame(time);
    }
    fn assert_click_effect(&self, before: &Node, after: &Node, handler: usize, side: &str) {
        let (width, height, scale) = self.viewport;
        let (width, height) = (width.max(2048.), height.max(2048.));
        // Two isolated renderer histories start with the exact same authored
        // tree. Observe the clicked history against the unclicked history at
        // equal timestamps. Normal motion makes keyframe toggles observable
        // even in a reduced-motion configuration; the real histories above
        // still prove exact parity with that configuration's motion policy.
        let idle = Renderer::new();
        let clicked = Renderer::new();
        idle.render_at(before, width, height, scale, 0., false);
        clicked.render_at(before, width, height, scale, 0., false);
        clicked.render_at(after, width, height, scale, 0., false);
        let changed = [0., 100., 350.].into_iter().any(|time| {
            serialized(&idle.render_at(before, width, height, scale, time, false))
                != serialized(&clicked.render_at(after, width, height, scale, time, false))
        });
        assert!(
            changed,
            "{}: {side} scripted click handler {handler} changed no scene",
            self.name
        );
    }
}
// Counts describe real authored handlers across all lesson states, including
// fade/menu close buttons and each item emitted by lists. Comments do not count.
fn expected_handlers(name: &str) -> usize {
    if let Some(lesson) = tour::LESSONS.iter().find(|lesson| lesson.id == name) {
        return lesson.handlers;
    }
    match name {
        "bindings" | "lists" => 3,
        "counter" | "comment-targets" | "fade" | "menu" | "transforms" | "late-handler"
        | "clipped-handler" => 2,
        "control-flow" | "functions" | "values" | "layout" | "layout-motion" | "grow" | "toast"
        | "keyframes" | "spring" | "presence" | "stagger" | "no-op" | "missing-target" => 1,
        "arrays" | "booleans" | "comments" | "components" | "decisions" | "first-function"
        | "hello-world" | "named-values" | "numbers" | "strings" => 0,
        _ => panic!("{name}: missing expected handler count"),
    }
}
impl Pair {
    fn exercise_discovered_handlers(&mut self, expected: usize) {
        let mut exercised = BTreeSet::new();
        for step in 0..64 {
            // Rust's routes are refreshed by tree(); compare retained inventories
            // even when the renderer omits a fully clipped target.
            let vm = handler_nodes(self.vm.tree());
            let rust = handler_nodes(&self.rust.tree());
            assert_eq!(vm, rust, "{}: retained handlers differ", self.name);
            let Some((id, handler)) = vm.into_iter().find(|(id, _)| !exercised.contains(id)) else {
                assert!(
                    exercised.len() >= expected,
                    "{}: exercised {} handlers, expected at least {expected}",
                    self.name,
                    exercised.len()
                );
                return;
            };
            exercised.insert(id);
            self.click(handler, 100. + step as f64 * 2000.);
            // The next iteration collects again after this event, so newly
            // mounted handlers enter the walk before any coverage assertion.
        }
        panic!("{}: handler discovery exceeded 64 clicks", self.name);
    }
}

fn parity(name: &'static str, source: &str, app: fn() -> UiApp, motion: bool) {
    for (viewport, reduced) in [
        ((560., 480., 1.), false),
        ((360., 640., 2.), false),
        ((560., 480., 1.), true),
    ] {
        let program = compiler::compile_entry(source, &Hosts::default(), "App").unwrap();
        let mut pair = Pair {
            name,
            vm: UiSession::new(program).unwrap(),
            rust: app(),
            vm_renderer: Renderer::new(),
            rust_renderer: Renderer::new(),
            viewport,
            reduced,
            comparisons: 0,
        };
        let (scene, _) = pair.frame(0.);
        let handlers: Vec<_> = scene
            .targets
            .iter()
            .map(|target| target.handler)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let expected = expected_handlers(name);
        assert!(
            expected == 0 || !handlers.is_empty(),
            "{name}: expected handlers but initial scene has no targets"
        );
        for action in tour::script(&handlers, motion).into_iter().skip(1) {
            match action {
                Action::Frame(time) => {
                    pair.frame(time);
                }
                Action::Click(handler, time) => pair.click(handler, time),
            }
        }
        // Start an independent coverage history so motion retarget scripts and
        // intentionally idempotent reset/selection buttons keep their ordering.
        let mut discovery = Pair {
            name,
            vm: UiSession::new(compiler::compile_entry(source, &Hosts::default(), "App").unwrap())
                .unwrap(),
            rust: app(),
            vm_renderer: Renderer::new(),
            rust_renderer: Renderer::new(),
            viewport,
            reduced,
            comparisons: 0,
        };
        discovery.frame(0.);
        discovery.exercise_discovered_handlers(expected);
        println!(
            "{name}: {:?}, reduced={reduced}, {} complete scene comparisons, every handler exercised",
            viewport,
            (pair.comparisons + discovery.comparisons)
        );
    }
}

#[test]
fn all_lessons_match_the_original_complete_scenes() {
    for lesson in tour::LESSONS {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../deka_fmt/tests/fixtures/tour")
            .join(format!("{}.dsx", lesson.id));
        parity(
            lesson.id,
            &std::fs::read_to_string(path).unwrap(),
            lesson.app,
            lesson.motion,
        );
    }
}

#[test]
fn comments_cannot_inflate_the_rendered_handler_inventory() {
    let source = include_str!("../../deka_fmt/tests/fixtures/tour/counter.dsx").to_owned()
        + "\n// onClick= comment, not a handler\n// onClick= another comment\n";
    parity(
        "comment-targets",
        &source,
        || {
            (tour::LESSONS
                .iter()
                .find(|l| l.id == "counter")
                .unwrap()
                .app)()
        },
        false,
    );
}

#[test]
#[should_panic(expected = "scripted click handler 0 changed no scene")]
fn matching_noop_handlers_on_both_sides_are_rejected() {
    parity(
        "no-op",
        "export fn App(){return (<view><button onClick={fn(){}}>Same</button></view>);}",
        || UiApp::new(|| deka_ui::view! {<view><button onClick={|_| {}}>"Same"</button></view>}),
        false,
    );
}

#[test]
#[should_panic(expected = "initial scene has no targets")]
fn authored_handlers_cannot_pass_with_an_empty_scene_inventory() {
    parity(
        "missing-target",
        "export fn App(){return (<view><view className=\"h-0 overflow-hidden\"><button onClick={fn(){}}>Hidden</button></view></view>);}",
        || {
            UiApp::new(
                || deka_ui::view! {<view><view className="h-0 overflow-hidden"><button onClick={|_| {}}>"Hidden"</button></view></view>},
            )
        },
        false,
    );
}
#[test]
#[should_panic(expected = "scripted click handler 1 changed no scene")]
fn newly_mounted_noop_handler_is_exercised() {
    parity(
        "late-handler",
        "export fn App(){let open=0;return (<view><button onClick={fn(){open=1-open;}}>Toggle</button>{open==1 ? <button onClick={fn(){}}>Late</button> : None}</view>);}",
        || {
            UiApp::new(|| {
                let open = deka_ui::signal(false);
                deka_ui::view! {<view><button onClick={move |_| open.toggle()}>"Toggle"</button>
                    {move || open.get().expect("open is live for the lesson").then(|| deka_ui::view!{<button onClick={|_| {}}>"Late"</button>})}
                </view>}
            })
        },
        false,
    );
}
#[test]
#[should_panic(expected = "scripted click handler 1 changed no scene")]
fn clipped_noop_handler_is_exercised() {
    parity(
        "clipped-handler",
        "export fn App(){let count=0;return (<view><button onClick={fn(){count+=1;}}>Count: {count}</button><view className=\"h-0 overflow-hidden\"><button onClick={fn(){}}>Hidden</button></view></view>);}",
        || {
            UiApp::new(|| {
                let mut count = deka_ui::signal(0);
                deka_ui::view! {<view><button onClick={move |_| count+=1}>"Count: {count}"</button>
                    <view className="h-0 overflow-hidden"><button onClick={|_| {}}>"Hidden"</button></view>
                </view>}
            })
        },
        false,
    );
}
