use deka_ui::prelude::*;

fn main() {
    let _ = view! { <view className="p-4"/> };
    let classes = "p-4";
    let _ = view! { <view className={classes}/> };
    let _ = view! { <view className={move || classes}/> };
}
