use deka_ui::prelude::*;
#[component]
fn Card(title: String) -> View { view! { <p>{title}</p> } }
fn main() {
    let _app = UiApp::new(|| {
        view! {
            <view>
                <Card
                    title={123}
                />
            </view>
        }
    });
}
