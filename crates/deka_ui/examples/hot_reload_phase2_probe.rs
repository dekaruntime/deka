//! Disk-edit and retained-handler probe; no display or OS window.
#[cfg(all(feature = "hot-reload", debug_assertions))]
mod probe {
    use deka_ui::prelude::*;
    use std::io::BufRead;
    #[derive(Clone)]
    struct Controls {
        show: Signal<bool>,
        rows: Signal<usize>,
        count: Signal<i32>,
    }
    #[component]
    fn Counter(id: String, title: String, step: i32, seed: i32) -> View {
        let mut count = signal(seed);
        view! {<button id={id} onClick={move |_| count += step}>"{title}: {count}"<span>" step {step}"</span></button>}
            .with_state(ComponentState::new(count))
    }
    #[component]
    fn Scalars(message: &'static str, enabled: bool, letter: char, ratio: f32, tiny: u8) -> View {
        view! {<span id="scalars" disabled={enabled}>"{message}|{enabled}|{letter}|{ratio}|{tiny}"</span>}
    }
    #[component]
    fn App() -> View {
        let show = signal(false);
        let rows = signal(2_usize);
        let mut count = signal(3);
        let conditional = move || {
            show.get().unwrap().then(|| view! {<button id="conditional" onClick={move |_| count += 1}>"Conditional: {count}"</button>})
        };
        let list = move || {
            (0..rows.get().unwrap())
                .map(|index| View::text(format!("row{index}")))
                .collect::<Vec<_>>()
        };
        let opaque = View::fragment([View::dynamic(move || {
            View::text(format!("Opaque: {}", count.get().unwrap()))
        })]);
        view! {<view><div id="left">{conditional}<span id="marker">"marker"</span>{list}<Counter id="counter" title="Old" step=2 seed=0/></div><div id="right"/><Scalars message="Static" enabled=true letter='x' ratio=1.5f32 tiny=1u8/><Scalars message="Other" enabled=true letter='x' ratio=1.5f32 tiny=1u8/><div id="fragment-left">{opaque}</div><div id="fragment-right"/></view>}
            .with_state(ComponentState::new(Controls { show, rows, count }))
    }
    fn text(node: &deka_native_ir::Node) -> String {
        node.text.clone().unwrap_or_default() + &node.children.iter().map(text).collect::<String>()
    }
    fn handler(node: &deka_native_ir::Node, prefix: &str) -> Option<String> {
        node.on_click
            .filter(|_| text(node).starts_with(prefix))
            .map(|_| node.id.clone())
            .or_else(|| {
                node.children
                    .iter()
                    .find_map(|child| handler(child, prefix))
            })
    }
    fn snapshot(node: &deka_native_ir::Node) -> serde_json::Value {
        serde_json::json!({"id":node.id,"text":node.text,"children":node.children.iter().map(snapshot).collect::<Vec<_>>()})
    }
    pub fn run() {
        let mut app = UiApp::new(App);
        let controls = app
            .tree()
            .query("view")
            .unwrap()
            .unwrap()
            .component_state::<Controls>()
            .unwrap();
        let counter = app.tree().get_element_by_id("counter").unwrap();
        counter.component_state::<Signal<i32>>().unwrap().set(7);
        let counter_handler = handler(&app.tree().snapshot(), "Old:").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                sender.send(line.unwrap()).unwrap();
            }
        });
        println!("READY pid={}", std::process::id());
        println!(
            "TREE {}",
            serde_json::to_string(&snapshot(&app.tree().snapshot())).unwrap()
        );
        loop {
            let status = app.poll_hot_reload();
            let mut changed = status != deka_ui::hot_reload::ReloadStatus::Unchanged;
            if let Ok(command) = receiver.try_recv() {
                match command.as_str() {
                    "show" => controls.show.set(true),
                    "hide" => controls.show.set(false),
                    "rows" => controls.rows.set(3),
                    "click" => {
                        assert!(app.dispatch_to(
                            &handler(&app.tree().snapshot(), "Conditional:").unwrap(),
                            Event::Click
                        ));
                    }
                    "counter" => assert!(app.dispatch_to(&counter_handler, Event::Click)),
                    _ => panic!("unknown probe command"),
                }
                changed = true;
            }
            if changed {
                assert!(controls.count.get().unwrap() >= 3);
                assert!(app.tree().get_element_by_id("counter").unwrap() == counter);
                println!("STATUS {status:?}");
                println!(
                    "TREE {}",
                    serde_json::to_string(&snapshot(&app.tree().snapshot())).unwrap()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
}
#[cfg(all(feature = "hot-reload", debug_assertions))]
fn main() {
    probe::run();
}
#[cfg(not(all(feature = "hot-reload", debug_assertions)))]
fn main() {}
