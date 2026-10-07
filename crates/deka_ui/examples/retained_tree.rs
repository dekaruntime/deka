use deka_ui::prelude::*;

#[derive(Clone)]
struct CounterState {
    count: Signal<i32>,
}

#[component]
pub fn App() -> View {
    let count = signal(0);
    let message = node_ref();
    view! {
        <view className="p-4 gap-3">
            <p id="message" node_ref={message.clone()}>"Count: {count}"</p>
            <button onClick={move |_| {
                let node = tree().unwrap().query("#message").unwrap().unwrap();
                node.set_text_content("Hand edit").unwrap();
                node.class_list().add("opacity-25").unwrap();
            }}>"Edit"</button>
            <button onClick={|_| {
                let node = tree().unwrap().query("#message").unwrap().unwrap();
                let mut count = node.component_state::<CounterState>().unwrap().count;
                count += 1;
            }}>"Add one"</button>
        </view>
    }
    .with_state(ComponentState::new(CounterState { count }))
}

#[cfg(not(test))]
fn main() {
    deka_ui::launch(App);
}
