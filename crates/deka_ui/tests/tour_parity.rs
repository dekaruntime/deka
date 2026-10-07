//! Complete scene equality against the unchanged DekaScript tour/VM reference.
use deka_native_ir::Node;
use deka_native_ui::scene::{Renderer, Scene};
use deka_ui::UiApp;
use deka_vm::{Hosts, compiler, ui::UiSession};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

#[path = "../examples/tour/arrays.rs"]
mod arrays;
#[path = "../examples/tour/bindings.rs"]
mod bindings;
#[path = "../examples/tour/booleans.rs"]
mod booleans;
#[path = "../examples/tour/comments.rs"]
mod comments;
#[path = "../examples/tour/components.rs"]
mod components;
#[path = "../examples/tour/control-flow.rs"]
mod control_flow;
#[path = "../examples/tour/counter.rs"]
mod counter;
#[path = "../examples/tour/decisions.rs"]
mod decisions;
#[path = "../examples/tour/fade.rs"]
mod fade;
#[path = "../examples/tour/first-function.rs"]
mod first_function;
#[path = "../examples/tour/functions.rs"]
mod functions;
#[path = "../examples/tour/grow.rs"]
mod grow;
#[path = "../examples/tour/hello-world.rs"]
mod hello_world;
#[path = "../examples/tour/keyframes.rs"]
mod keyframes;
#[path = "../examples/tour/layout.rs"]
mod layout;
#[path = "../examples/tour/layout-motion.rs"]
mod layout_motion;
#[path = "../examples/tour/lists.rs"]
mod lists;
#[path = "../examples/tour/menu.rs"]
mod menu;
#[path = "../examples/tour/named-values.rs"]
mod named_values;
#[path = "../examples/tour/numbers.rs"]
mod numbers;
#[path = "../examples/tour/presence.rs"]
mod presence;
#[path = "../examples/tour/spring.rs"]
mod spring;
#[path = "../examples/tour/stagger.rs"]
mod stagger;
#[path = "../examples/tour/strings.rs"]
mod strings;
#[path = "../examples/tour/toast.rs"]
mod toast;
#[path = "../examples/tour/transforms.rs"]
mod transforms;
#[path = "../examples/tour/values.rs"]
mod values;

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
    let expected: BTreeSet<_> = LESSONS.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(expected.len(), 27);
    assert_eq!(
        inventory(&root.join("../deka_fmt/tests/fixtures/tour"), "dsx"),
        expected
    );
    assert_eq!(inventory(&root.join("examples/tour"), "rs"), expected);
    let manifest = include_str!("../Cargo.toml");
    for name in LESSONS {
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
        let point = scene
            .targets
            .iter()
            .find(|target| target.handler == handler)
            .and_then(|target| target.rect.intersection(target.clip))
            .and_then(|rect| {
                (0..10)
                    .flat_map(|y| {
                        (0..10).map(move |x| {
                            (
                                rect.x + rect.width * (x as f32 + 0.5) / 10.,
                                rect.y + rect.height * (y as f32 + 0.5) / 10.,
                            )
                        })
                    })
                    .find(|&(x, y)| scene.hit(x, y).is_some_and(|hit| hit.handler == handler))
            });
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
fn handler_nodes(node: &Node) -> Vec<(String, usize)> {
    fn visit(node: &Node, output: &mut Vec<(String, usize)>) {
        if let Some(handler) = node.on_click {
            output.push((node.id.clone(), handler));
        }
        for child in &node.children {
            visit(child, output);
        }
    }
    let mut output = vec![];
    visit(node, &mut output);
    output
}
// Counts describe real authored handlers across all lesson states, including
// fade/menu close buttons and each item emitted by lists. Comments do not count.
fn expected_handlers(name: &str) -> usize {
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
        pair.frame(16.);
        if !handlers.is_empty() {
            if motion {
                pair.click(handlers[0], 100.);
                for t in [125., 220.] {
                    pair.frame(t);
                }
                pair.click(handlers[0], 220.); // Retarget before the first motion finishes.
                for t in [225., 350., 600., 1200., 2000.] {
                    pair.frame(t);
                }
                pair.click(handlers[0], 2000.); // Re-enter after exit, preserving identity rules.
                for t in [2120., 2240., 2500., 3400., 4000.] {
                    pair.frame(t);
                }
                for &handler in handlers.iter().skip(1) {
                    let time = 5000. + handler as f64 * 2000.;
                    pair.click(handler, time);
                    for delta in [120., 350., 1000.] {
                        pair.frame(time + delta);
                    }
                }
            } else {
                // Selection lessons have intentionally idempotent buttons.
                // Alternate targets instead of scripting same-value clicks.
                for (step, &handler) in handlers
                    .iter()
                    .cycle()
                    .take(2 * handlers.len() + 2)
                    .enumerate()
                {
                    pair.click(handler, 100. + step as f64 * 100.);
                }
            }
        }
        pair.frame(20000.);
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

macro_rules! lessons {
    ($($name:ident: $file:literal => $motion:expr),* $(,)?) => {
        const LESSONS: &[&str] = &[$($file),*];
        $(#[test]
        fn $name() {
            parity($file, include_str!(concat!("../../deka_fmt/tests/fixtures/tour/",$file,".dsx")), || UiApp::new($name::App), $motion);
        })*
    };
}
lessons! {
    arrays: "arrays" => false,
    bindings: "bindings" => false,
    booleans: "booleans" => false,
    comments: "comments" => false,
    components: "components" => false,
    control_flow: "control-flow" => false,
    counter: "counter" => false,
    decisions: "decisions" => false,
    fade: "fade" => true,
    first_function: "first-function" => false,
    functions: "functions" => false,
    grow: "grow" => true,
    hello_world: "hello-world" => false,
    keyframes: "keyframes" => true,
    layout_motion: "layout-motion" => true,
    layout: "layout" => false,
    lists: "lists" => false,
    menu: "menu" => true,
    named_values: "named-values" => false,
    numbers: "numbers" => false,
    presence: "presence" => true,
    spring: "spring" => true,
    stagger: "stagger" => true,
    strings: "strings" => false,
    toast: "toast" => true,
    transforms: "transforms" => true,
    values: "values" => false,
}

#[test]
fn comments_cannot_inflate_the_rendered_handler_inventory() {
    let source = include_str!("../../deka_fmt/tests/fixtures/tour/counter.dsx").to_owned()
        + "\n// onClick= comment, not a handler\n// onClick= another comment\n";
    parity(
        "comment-targets",
        &source,
        || UiApp::new(counter::App),
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
                    {move || open.get().then(|| deka_ui::view!{<button onClick={|_| {}}>"Late"</button>})}
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
