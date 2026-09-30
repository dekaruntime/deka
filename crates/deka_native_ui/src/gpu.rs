use crate::{Application as NativeApplication, Host, Node};
use gpui::{prelude::*, *};
use std::{collections::HashMap, time::Duration};

struct View<A: NativeApplication> {
    host: Host<A>,
    focus: HashMap<String, FocusHandle>,
}
impl<A: NativeApplication> View<A> {
    fn element(&mut self, node: Node, cx: &mut Context<Self>) -> AnyElement {
        let style = node.style;
        let mut el = div()
            .id(SharedString::from(node.id.clone()))
            .flex()
            .p(px(style.padding))
            .gap(px(style.gap))
            .rounded(px(style.radius));
        el = if style.row {
            el.flex_row()
        } else {
            el.flex_col()
        };
        if let Some(color) = style.background {
            el = el.bg(rgb(color));
        }
        if let Some(color) = style.color {
            el = el.text_color(rgb(color));
        }
        if let Some(size) = style.font_size {
            el = el.text_size(px(size));
        }
        if let Some(width) = style.width {
            el = el.w(px(width));
        }
        if let Some(height) = style.height {
            el = el.h(px(height));
        }
        if let Some(text) = node.text {
            el = el.child(text);
        }
        if let Some(handler) = node.on_click {
            let focus = self
                .focus
                .entry(node.id)
                .or_insert_with(|| cx.focus_handle())
                .clone();
            el = el
                .track_focus(&focus)
                .tab_index(0)
                .focus(|style| style.border_2().border_color(rgb(0x0c8b43)))
                .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&focus))
                .cursor_pointer()
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.host.click(handler);
                    cx.notify();
                }))
                .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        view.host.click(handler);
                        cx.stop_propagation();
                        cx.notify();
                    }
                }));
        }
        for child in node.children {
            el = el.child(self.element(child, cx));
        }
        el.into_any_element()
    }
}
impl<A: NativeApplication> Render for View<A> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = self.host.render();
        fn ids(node: &Node, set: &mut std::collections::HashSet<String>) {
            set.insert(node.id.clone());
            for child in &node.children {
                ids(child, set);
            }
        }
        let mut active = std::collections::HashSet::new();
        ids(&root, &mut active);
        self.focus.retain(|id, _| active.contains(id));
        let background = root.style.background.unwrap_or(0xffffff);
        div()
            .size_full()
            .bg(rgb(background))
            .text_color(rgb(0x1a1611))
            .text_size(px(16.))
            .child(self.element(root, cx))
    }
}
pub fn run<A: NativeApplication>(app: A) {
    // This mode exercises the same application logic without opening a window.
    let args: Vec<_> = std::env::args().collect();
    if let Some(index) = args.iter().position(|a| a == "--exercise") {
        let clicks = args
            .get(index + 1)
            .and_then(|s| s.parse().ok())
            .expect("--exercise requires a nonnegative click count");
        println!("{}", crate::exercise(app, clicks));
        return;
    }
    let live = app.live();
    gpui::Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(560.), px(300.)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Deka native experiment".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    if live {
                        cx.spawn(async move |view, cx| {
                            loop {
                                cx.background_executor()
                                    .timer(Duration::from_millis(100))
                                    .await;
                                if view
                                    .update(cx, |view: &mut View<A>, cx| {
                                        if view.host.refresh() {
                                            cx.notify();
                                        }
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                    }
                    View {
                        host: Host::new(app),
                        focus: HashMap::new(),
                    }
                })
            },
        )
        .expect("open native window");
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
