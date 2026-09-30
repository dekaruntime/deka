//! Emit native-host scenes for the website's WASM parity check.
use deka_native_web::PortfolioWorld;
use serde_json::json;
fn main() {
    let mut world = PortfolioWorld::new();
    world.start();
    let mut frames = vec![];
    for frame in 0..=240 {
        let key = match frame {
            0 => Some(("ArrowUp", true)),
            90 => Some(("ArrowUp", false)),
            91 => Some(("Enter", true)),
            92 => Some(("Enter", false)),
            160 => Some(("Enter", true)),
            161 => Some(("Enter", false)),
            _ => None,
        };
        if let Some((key, down)) = key {
            world.key(key, down);
        }
        let time = frame as f64 * 1000. / 120.;
        let scene = world.frame(960., 640., time, false);
        let sounds = world.sounds();
        if [0, 60, 90, 100, 120, 150, 180, 220, 240].contains(&frame) {
            frames.push(json!({"frame":frame,"state":serde_json::from_str::<serde_json::Value>(&world.snapshot()).unwrap(),"scene":serde_json::from_str::<serde_json::Value>(&scene).unwrap(),"sounds":sounds}));
        }
    }
    println!("{}", serde_json::to_string(&frames).unwrap());
}
