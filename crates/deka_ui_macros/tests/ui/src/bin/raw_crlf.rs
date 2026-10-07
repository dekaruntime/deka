use deka_ui::view;
fn main() {
    let _ = view! { <p>r#"First
prefix {bad + expr}"#</p> };
}
