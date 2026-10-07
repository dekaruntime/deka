use deka_ui::prelude::*;
fn main() {
    let _app = UiApp::new(|| {
        view! {
            <view>
                <Missing title="Unknown" />
            </view>
        }
    });
}
