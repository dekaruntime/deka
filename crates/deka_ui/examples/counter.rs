use deka_ui::prelude::*;

fn app() -> View {
    let mut count = signal(0);
    View::element("view")
        .attr("className", "p-4 gap-3")
        .child(View::element("p").child(View::live_text(move || format!("Count: {count}"))))
        .child(
            View::element("button")
                .on_click(move |_| count += 1)
                .child("Add one"),
        )
}
fn main() {
    if std::env::args().any(|arg| arg == "--smoke") {
        let mut options = WindowOptions::new("Deka Rust counter", 560., 300.);
        options.frames = Some(1);
        options.on_frame = Some(Box::new(|frame| {
            println!("Presented Rust counter frame {}", frame.index)
        }));
        deka_ui::launch_with(app, options, false);
    } else {
        deka_ui::launch(app);
    }
}
