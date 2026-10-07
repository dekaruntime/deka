use deka_ui::prelude::*;

fn total(values: &[i32]) -> i32 {
    let mut result = 0;
    for value in values.iter().take(3) {
        result += value;
    }
    result
}

#[component]
pub fn App() -> View {
    let values = [3, 5, 8];
    let open = signal(false);
    view! {<view className="p-6 gap-3 bg-[#F3EFE3] text-[#1A1611]">
        <p className="text-xl">"Loops and conditions"</p>
        <p>"Total: "{total(&values)}</p>
        <button onClick={move |_| open.toggle()}>"Toggle answer"</button>
        {move || open.get().unwrap_or_default().then(|| view!{<p>"The total is 16."</p>})}
    </view>}
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
