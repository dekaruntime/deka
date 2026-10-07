use deka_ui::prelude::*;
fn main() {
    let _app = UiApp::new(|| {
        view! { <p>"Text before error: {a.b}"</p> }
    });
}
