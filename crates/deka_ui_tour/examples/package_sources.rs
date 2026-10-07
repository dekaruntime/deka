//! Build inputs and browser expectations come from the same apps/scripts as parity.
use deka_native_ui::scene::{Renderer, Scene};
use deka_ui::tour::{self, Action};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
fn contract(scene: &Scene) -> Value {
    let mut value = serde_json::to_value(scene).unwrap();
    for image in value["images"].as_array_mut().unwrap() {
        let hash = image["rgba"]
            .as_array()
            .unwrap()
            .iter()
            .fold(5381u32, |h, byte| {
                h.wrapping_mul(33) ^ byte.as_u64().unwrap() as u32
            });
        image.as_object_mut().unwrap().remove("rgba");
        image["hash"] = json!(hash);
    }
    value["images"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(|image| image["id"].as_str().unwrap().to_owned());
    value
}
fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&out).unwrap();
    let sources:Vec<_>=tour::LESSONS.iter().map(|lesson|json!({"id":lesson.id,"title":lesson.title,"section":lesson.section,"content":lesson.description,"path":format!("crates/deka_ui/examples/tour/{}.rs",lesson.id),"source":lesson.source,"handlers":lesson.handlers})).collect();
    std::fs::write(
        out.join("sources.json"),
        serde_json::to_vec(&sources).unwrap(),
    )
    .unwrap();
    let mut cases = vec![];
    for lesson in tour::LESSONS {
        for (width, height, scale, reduced) in [
            (560., 480., 1., false),
            (360., 640., 2., false),
            (560., 480., 1., true),
        ] {
            for discovery in [false, true] {
                let app = (lesson.app)();
                let renderer = Renderer::new();
                let frame =
                    |time| renderer.render_at(&app.tree(), width, height, scale, time, reduced);
                let initial = frame(0.);
                let handlers: Vec<_> = initial
                    .targets
                    .iter()
                    .map(|t| t.handler)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let mut steps = vec![];
                let mut actions = tour::script(&handlers, lesson.motion).into_iter();
                let mut seen = BTreeSet::new();
                let mut coverage = 0;
                loop {
                    let action = if discovery {
                        let Some((id, handler)) = tour::handler_nodes(&app.tree())
                            .into_iter()
                            .find(|(id, _)| !seen.contains(id))
                        else {
                            assert!(coverage >= lesson.handlers);
                            break;
                        };
                        seen.insert(id);
                        let time = 100. + coverage as f64 * 2000.;
                        coverage += 1;
                        assert!(coverage <= 64, "discovery must terminate");
                        Action::Click(handler, time)
                    } else {
                        let Some(action) = actions.next() else { break };
                        action
                    };
                    match action {
                        Action::Frame(time) => {
                            steps.push(json!({"time":time,"scene":contract(&frame(time))}))
                        }
                        Action::Click(handler, time) => {
                            let before = frame(time);
                            let point = tour::click_point(&before, handler)
                                .expect("browser click must be visible");
                            assert!(app.dispatch(handler));
                            steps.push(json!({"time":time,"point":point,"before":contract(&before),"scene":contract(&frame(time))}));
                        }
                    }
                }
                cases.push(json!({"lesson":lesson.id,"width":width,"height":height,"scale":scale,"reduced":reduced,"discovery":discovery,"steps":steps}));
            }
        }
    }
    std::fs::write(
        out.join("expectations.json"),
        serde_json::to_vec(&cases).unwrap(),
    )
    .unwrap();
    println!(
        "Packaged {} sources and {} browser histories",
        sources.len(),
        cases.len()
    );
}
