//! The first Rust component set, sharing the retained tree and native renderer.
use crate::{Event, EventKind, Signal, View, component, derived, node_ref, signal};
use std::rc::Rc;

/// Explicit theme selection. A shared signal switches already mounted controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}
/// Semantic colours from zega.dev's light/dark palette.
#[derive(Clone, Copy, Debug)]
pub struct ThemeTokens {
    pub background: &'static str,
    pub surface: &'static str,
    pub foreground: &'static str,
    pub muted: &'static str,
    pub accent: &'static str,
    pub accent_foreground: &'static str,
}
impl Theme {
    pub const fn tokens(self) -> ThemeTokens {
        match self {
            Self::Light => ThemeTokens {
                background: "#f3efe3",
                surface: "#fbf8f0",
                foreground: "#1a1611",
                muted: "#6a5b4c",
                accent: "#0c8b43",
                accent_foreground: "#ffffff",
            },
            Self::Dark => ThemeTokens {
                background: "#1e1610",
                surface: "#2a2018",
                foreground: "#f5efe2",
                muted: "#a8937c",
                accent: "#3ccb77",
                accent_foreground: "#1a1611",
            },
        }
    }
    pub fn surface_class(self) -> String {
        let t = self.tokens();
        format!("bg-[{}] text-[{}]", t.surface, t.foreground)
    }
    pub fn page_class(self) -> String {
        let t = self.tokens();
        format!("bg-[{}] text-[{}]", t.background, t.foreground)
    }
    fn action_class(self, selected: bool) -> String {
        let t = self.tokens();
        if selected {
            format!("bg-[{}] text-[{}]", t.accent, t.accent_foreground)
        } else {
            self.surface_class()
        }
    }
}
/// Renderer-owned motion. Hosts supply their reduced-motion preference to frames.
#[derive(Clone, Copy, Debug)]
pub enum MotionPreset {
    Enter,
    Exit,
    Press,
}
impl MotionPreset {
    pub const fn classes(self) -> &'static str {
        match self {
            Self::Enter => "enter-fade duration-160 ease-out",
            Self::Exit => "exit-fade duration-160 ease-out",
            Self::Press => "frames-scale-[0:1,40:0.96,100:1] repeat-1 duration-120",
        }
    }
    fn activation(sequence: u64) -> &'static str {
        match sequence {
            0 => "",
            n if n % 2 == 1 => Self::Press.classes(),
            // Alternate the timeline to restart an already mounted animation
            // on every activation, including clicks during the previous press.
            _ => "frames-scale-[0:1,45:0.96,100:1] repeat-1 duration-120",
        }
    }
}
fn identified(view: View, id: String) -> View {
    if id.is_empty() {
        view
    } else {
        view.attr("id", id)
    }
}

/// A labelled action, activated by pointer, Enter/Space, or accessibility.
#[component]
pub fn Button(
    label: String,
    on_press: Rc<dyn Fn()>,
    #[prop(default)] id: String,
    #[prop(default = signal(Theme::Light))] theme: Signal<Theme>,
    #[prop(default = signal(false))] disabled: Signal<bool>,
) -> View {
    let presses = signal(0u64);
    let view = View::element("button")
        .attr("aria-label", label.clone())
        .attr("disabled", move || disabled.get().unwrap_or(true))
        .attr("class", move || {
            format!(
                "px-4 py-2 rounded-lg {} {} {}",
                theme.get().unwrap_or_default().action_class(true),
                if disabled.get().unwrap_or(true) {
                    "opacity-50"
                } else {
                    "opacity-100"
                },
                MotionPreset::activation(presses.get().unwrap_or_default())
            )
        })
        .on_click(move |_| {
            if !disabled.get().unwrap_or(true) {
                presses.set(presses.get().unwrap_or_default().wrapping_add(1).max(1));
                on_press();
            }
        })
        .child(label);
    identified(view, id)
}

/// A controlled single-line native editor. The host owns selection, IME,
/// clipboard and focus; edits publish through the supplied string signal.
#[component]
pub fn Input(
    label: String,
    value: Signal<String>,
    #[prop(default)] id: String,
    #[prop(default)] placeholder: String,
    #[prop(default = signal(Theme::Light))] theme: Signal<Theme>,
    #[prop(default = signal(false))] disabled: Signal<bool>,
) -> View {
    identified(
        View::element("input")
            .attr("aria-label", label)
            .attr("placeholder", placeholder)
            .attr("disabled", move || disabled.get().unwrap_or(true))
            .attr("class", move || {
                format!(
                    "w-full h-10 rounded-lg {} {}",
                    theme.get().unwrap_or_default().surface_class(),
                    if disabled.get().unwrap_or(true) {
                        "opacity-50"
                    } else {
                        "opacity-100"
                    }
                )
            })
            .value(value),
        id,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub key: String,
    pub label: String,
    pub disabled: bool,
}
impl ListItem {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            disabled: false,
        }
    }
}
/// Keyed selectable rows. Unique keys preserve nodes/handlers through reorder;
/// labels and disabled state remain signal bindings. Duplicate keys fail closed.
#[component]
pub fn List(
    label: String,
    items: Signal<Vec<ListItem>>,
    selected: Signal<Option<String>>,
    #[prop(default)] id: String,
    #[prop(default = signal(Theme::Light))] theme: Signal<Theme>,
) -> View {
    // Reorders don't change this shared lookup. Row bindings clone one Rc,
    // rather than cloning and scanning the full collection for every field.
    let indexed = derived(move || {
        Rc::new(
            items
                .get()
                .unwrap_or_default()
                .into_iter()
                .map(|item| (item.key.clone(), item))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
    });
    let rows = View::keyed(
        move || {
            items
                .get()
                .unwrap_or_default()
                .into_iter()
                .map(|item| item.key)
                .collect()
        },
        move |key| {
            let text_key = key.clone();
            let disabled_key = key.clone();
            let style_key = key.clone();
            View::element("div").attr("role", "listitem").child(
                View::element("button")
                    .attr("class", move || {
                        format!(
                            "w-full px-4 py-2 rounded-lg {}",
                            theme.get().unwrap_or_default().action_class(
                                selected.get().unwrap_or_default().as_ref() == Some(&style_key)
                            )
                        )
                    })
                    .attr("disabled", move || {
                        indexed
                            .get()
                            .unwrap_or_default()
                            .get(&disabled_key)
                            .is_none_or(|item| item.disabled)
                    })
                    .on_click(move |_| {
                        selected.set(Some(key.clone()));
                    })
                    .child(View::live_text(move || {
                        indexed
                            .get()
                            .unwrap_or_default()
                            .get(&text_key)
                            .map(|item| item.label.clone())
                            .unwrap_or_default()
                    })),
            )
        },
    );
    identified(
        rows.attr("role", "list")
            .attr("aria-label", label)
            .attr("class", "gap-2"),
        id,
    )
}

/// One tab and its panel factory. Only the active panel is mounted; switching
/// disposes panel-local state. Keep persistent state in the parent signals.
#[derive(Clone)]
pub struct TabItem {
    pub key: String,
    pub label: String,
    pub disabled: bool,
    pub panel: Rc<dyn Fn() -> View>,
}
impl TabItem {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        panel: impl Fn() -> View + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            disabled: false,
            panel: Rc::new(panel),
        }
    }
}

/// Horizontal tabs with automatic activation, roving focus, wraparound arrows,
/// Home/End, disabled skipping and labelled panels. IDs must be unique per app.
#[component]
pub fn Tabs(
    id: String,
    label: String,
    items: Vec<TabItem>,
    selected: Signal<String>,
    #[prop(default = signal(Theme::Light))] theme: Signal<Theme>,
) -> View {
    let mut unique = std::collections::BTreeSet::new();
    if id.is_empty()
        || items
            .iter()
            .any(|item| item.key.is_empty() || !unique.insert(item.key.clone()))
    {
        // Use the app's ordinary mount-error path, including its configured sink.
        return View::error("Tabs require a nonempty id and unique nonempty keys");
    }
    let items = Rc::new(items);
    let active_items = items.clone();
    let active = derived(move || {
        let key = selected.get().unwrap_or_default();
        active_items
            .iter()
            .find(|item| !item.disabled && item.key == key)
            .or_else(|| active_items.iter().find(|item| !item.disabled))
            .map(|item| item.key.clone())
            .unwrap_or_default()
    });
    let references = Rc::new(items.iter().map(|_| node_ref()).collect::<Vec<_>>());
    let tabs = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let key = item.key.clone();
            let active_key = key.clone();
            let style_key = key.clone();
            let tab_key = key.clone();
            let key_items = items.clone();
            let key_refs = references.clone();
            View::element("button")
                .attr("id", format!("{id}-tab-{index}"))
                .attr("role", "tab")
                .attr("aria-label", &item.label)
                .attr("aria-controls", format!("{id}-panel-{index}"))
                .attr("aria-selected", move || {
                    active.get().unwrap_or_default() == active_key
                })
                .attr("tabIndex", move || {
                    if active.get().unwrap_or_default() == tab_key {
                        0
                    } else {
                        -1
                    }
                })
                .attr("disabled", item.disabled)
                .attr("class", move || {
                    format!(
                        "px-4 py-2 rounded-lg {}",
                        theme
                            .get()
                            .unwrap_or_default()
                            .action_class(active.get().unwrap_or_default() == style_key)
                    )
                })
                .node_ref(references[index].clone())
                .on_click(move |_| selected.set(key.clone()))
                .on(EventKind::KeyDown, move |event| {
                    let Event::KeyDown(key) = event else { return };
                    let enabled: Vec<_> = key_items
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| !item.disabled)
                        .map(|(i, _)| i)
                        .collect();
                    let Some(position) = enabled.iter().position(|i| *i == index) else {
                        return;
                    };
                    let next = match key.as_str() {
                        "ArrowRight" => enabled[(position + 1) % enabled.len()],
                        "ArrowLeft" => enabled[(position + enabled.len() - 1) % enabled.len()],
                        "Home" => enabled[0],
                        "End" => *enabled.last().unwrap(),
                        "Enter" | " " => index,
                        _ => return,
                    };
                    selected.set(key_items[next].key.clone());
                    if let Some(node) = key_refs[next].get().and_then(|node| node.as_element()) {
                        // Attached refs are same-session handles. Host validates
                        // effective availability before applying the request.
                        if let Err(error) = node.focus() {
                            eprintln!("deka tabs focus: {error}");
                        }
                    }
                })
                .child(item.label.clone())
        })
        .collect::<Vec<_>>();
    let panels = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let key = item.key.clone();
            let hidden_key = key.clone();
            let style_key = key.clone();
            let panel = item.panel.clone();
            View::element("div")
                .attr("id", format!("{id}-panel-{index}"))
                .attr("role", "tabpanel")
                .attr("aria-label", &item.label)
                .attr("aria-hidden", move || {
                    active.get().unwrap_or_default() != hidden_key
                })
                .attr("class", move || {
                    if active.get().unwrap_or_default() == style_key {
                        format!(
                            "p-4 gap-3 rounded-lg {}",
                            theme.get().unwrap_or_default().surface_class()
                        )
                    } else {
                        "h-0 p-0 overflow-hidden shrink-0".into()
                    }
                })
                .child(View::dynamic(move || {
                    (active.get().unwrap_or_default() == key).then(|| panel())
                }))
        })
        .collect::<Vec<_>>();
    View::element("div")
        .attr("id", id)
        .attr("class", "gap-3")
        .child(
            View::element("div")
                .attr("role", "tablist")
                .attr("aria-label", label)
                .attr("class", "flex-row gap-2")
                .child(tabs),
        )
        // Hidden zero-height panels must not add inter-panel layout gaps.
        .child(View::element("div").child(panels))
}
