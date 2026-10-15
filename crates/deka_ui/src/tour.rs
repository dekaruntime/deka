#[path = "../examples/tour/arrays.rs"]
mod arrays;
#[path = "../examples/tour/bindings.rs"]
mod bindings;
#[path = "../examples/tour/booleans.rs"]
mod booleans;
#[path = "../examples/tour/comments.rs"]
mod comments;
#[path = "../examples/tour/components.rs"]
mod components;
#[path = "../examples/tour/control-flow.rs"]
mod control_flow;
#[path = "../examples/tour/counter.rs"]
pub mod counter;
#[path = "../examples/tour/decisions.rs"]
mod decisions;
#[path = "../examples/tour/fade.rs"]
mod fade;
#[path = "../examples/tour/first-function.rs"]
mod first_function;
#[path = "../examples/tour/functions.rs"]
mod functions;
#[path = "../examples/tour/grow.rs"]
mod grow;
#[path = "../examples/tour/hello-world.rs"]
mod hello_world;
#[path = "../examples/tour/keyframes.rs"]
mod keyframes;
#[path = "../examples/tour/layout.rs"]
pub mod layout;
#[path = "../examples/tour/layout-motion.rs"]
mod layout_motion;
#[path = "../examples/tour/lists.rs"]
mod lists;
#[path = "../examples/tour/menu.rs"]
mod menu;
#[path = "../examples/tour/named-values.rs"]
mod named_values;
#[path = "../examples/tour/numbers.rs"]
mod numbers;
#[path = "../examples/tour/presence.rs"]
mod presence;
#[path = "../examples/tour/spring.rs"]
mod spring;
#[path = "../examples/tour/stagger.rs"]
mod stagger;
#[path = "../examples/tour/strings.rs"]
mod strings;
#[path = "../examples/tour/toast.rs"]
mod toast;
#[path = "../examples/tour/transforms.rs"]
mod transforms;
#[path = "../examples/tour/values.rs"]
mod values;

/// One source and constructor per lesson, shared by the native gate and browser.
pub struct Lesson {
    pub title: &'static str,
    pub section: &'static str,
    pub description: &'static str,
    pub id: &'static str,
    pub source: &'static str,
    pub app: fn() -> crate::UiApp,
    pub motion: bool,
    pub handlers: usize,
}
pub static LESSONS: &[Lesson] = &[
    Lesson {
        title: "Hello, world!",
        section: "Getting started",
        description: "A component is a Rust function returning a View. The view! macro builds markup; quoted text becomes a paragraph.",
        id: "hello-world",
        source: include_str!("../examples/tour/hello-world.rs"),
        app: || crate::UiApp::new(hello_world::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Comments",
        section: "Getting started",
        description: "Rust comments explain the code without adding anything to the view.",
        id: "comments",
        source: include_str!("../examples/tour/comments.rs"),
        app: || crate::UiApp::new(comments::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Giving values names",
        section: "Getting started",
        description: "Rust let binds a value. Quoted text inside view! can interpolate a named value with braces.",
        id: "named-values",
        source: include_str!("../examples/tour/named-values.rs"),
        app: || crate::UiApp::new(named_values::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Numbers and calculations",
        section: "Getting started",
        description: "Rust evaluates arithmetic expressions before the view is mounted.",
        id: "numbers",
        source: include_str!("../examples/tour/numbers.rs"),
        app: || crate::UiApp::new(numbers::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Working with text",
        section: "Getting started",
        description: "Use Rust strings and formatting to build the text displayed in a view.",
        id: "strings",
        source: include_str!("../examples/tour/strings.rs"),
        app: || crate::UiApp::new(strings::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "True and false",
        section: "Getting started",
        description: "A bool has two values: true and false. Rust if expressions can choose the displayed message.",
        id: "booleans",
        source: include_str!("../examples/tour/booleans.rs"),
        app: || crate::UiApp::new(booleans::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Making decisions",
        section: "Getting started",
        description: "Use an if statement to choose which value the component displays.",
        id: "decisions",
        source: include_str!("../examples/tour/decisions.rs"),
        app: || crate::UiApp::new(decisions::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Lists of values",
        section: "Getting started",
        description: "Arrays keep ordered values. Rust indexes and iterators give access to their contents.",
        id: "arrays",
        source: include_str!("../examples/tour/arrays.rs"),
        app: || crate::UiApp::new(arrays::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Your first function",
        section: "Getting started",
        description: "A typed Rust function takes an input and returns a value. App can call ordinary Rust functions.",
        id: "first-function",
        source: include_str!("../examples/tour/first-function.rs"),
        app: || crate::UiApp::new(first_function::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Values and bindings",
        section: "Language",
        description: "A signal stores changing state. Click the button to update it; the bound text is patched in the retained tree.",
        id: "values",
        source: include_str!("../examples/tour/values.rs"),
        app: || crate::UiApp::new(values::App),
        motion: false,
        handlers: 1,
    },
    Lesson {
        title: "Functions",
        section: "Language",
        description: "Rust functions calculate values used in live text bindings. Click to change the signal input.",
        id: "functions",
        source: include_str!("../examples/tour/functions.rs"),
        app: || crate::UiApp::new(functions::App),
        motion: false,
        handlers: 1,
    },
    Lesson {
        title: "Control flow",
        section: "Language",
        description: "Rust loops calculate values. A live closure returns a view or None as state changes.",
        id: "control-flow",
        source: include_str!("../examples/tour/control-flow.rs"),
        app: || crate::UiApp::new(control_flow::App),
        motion: false,
        handlers: 1,
    },
    Lesson {
        title: "Bindings without React",
        section: "Views",
        description: "Components run once. Signals update only the bound properties that read them. Try the three buttons.",
        id: "bindings",
        source: include_str!("../examples/tour/bindings.rs"),
        app: || crate::UiApp::new(bindings::App),
        motion: false,
        handlers: 3,
    },
    Lesson {
        title: "Counter",
        section: "Views",
        description: "Click Add one or Subtract one. Event closures change a signal, and the bound count updates.",
        id: "counter",
        source: include_str!("../examples/tour/counter.rs"),
        app: || crate::UiApp::new(counter::App),
        motion: false,
        handlers: 2,
    },
    Lesson {
        title: "Reusable components",
        section: "Views",
        description: "The component attribute creates typed props. The same Card function supplies both cards.",
        id: "components",
        source: include_str!("../examples/tour/components.rs"),
        app: || crate::UiApp::new(components::App),
        motion: false,
        handlers: 0,
    },
    Lesson {
        title: "Lists and selection",
        section: "Views",
        description: "Map a Rust array into buttons. Each closure captures its project and updates the selected heading and style.",
        id: "lists",
        source: include_str!("../examples/tour/lists.rs"),
        app: || crate::UiApp::new(lists::App),
        motion: false,
        handlers: 3,
    },
    Lesson {
        title: "Layout basics",
        section: "Layout",
        description: "Rust computes flex layout and clipping. Resize the preview or toggle clipping to see the result.",
        id: "layout",
        source: include_str!("../examples/tour/layout.rs"),
        app: || crate::UiApp::new(layout::App),
        motion: false,
        handlers: 1,
    },
    Lesson {
        title: "Modal fade",
        section: "Motion",
        description: "Toggle the modal to animate opacity. Click Close to reverse the transition; Rust owns its presentation.",
        id: "fade",
        source: include_str!("../examples/tour/fade.rs"),
        app: || crate::UiApp::new(fade::App),
        motion: true,
        handlers: 2,
    },
    Lesson {
        title: "Sliding menu",
        section: "Motion",
        description: "Toggle the menu. Rust animates translation and keeps hit testing aligned with the presented position.",
        id: "menu",
        source: include_str!("../examples/tour/menu.rs"),
        app: || crate::UiApp::new(menu::App),
        motion: true,
        handlers: 2,
    },
    Lesson {
        title: "Growing button",
        section: "Motion",
        description: "Click the button to animate size and color. Nearby content follows the changing layout.",
        id: "grow",
        source: include_str!("../examples/tour/grow.rs"),
        app: || crate::UiApp::new(grow::App),
        motion: true,
        handlers: 1,
    },
    Lesson {
        title: "Toast notification",
        section: "Motion",
        description: "Toggle a notification combining opacity and translation. The button controls its presence.",
        id: "toast",
        source: include_str!("../examples/tour/toast.rs"),
        app: || crate::UiApp::new(toast::App),
        motion: true,
        handlers: 1,
    },
    Lesson {
        title: "Spring motion",
        section: "Motion",
        description: "Click while the box moves. A spring follows its destination with position and velocity.",
        id: "spring",
        source: include_str!("../examples/tour/spring.rs"),
        app: || crate::UiApp::new(spring::App),
        motion: true,
        handlers: 1,
    },
    Lesson {
        title: "Keyframes",
        section: "Motion",
        description: "Toggle a repeating keyframe timeline. The system reduced-motion preference stops continuous movement.",
        id: "keyframes",
        source: include_str!("../examples/tour/keyframes.rs"),
        app: || crate::UiApp::new(keyframes::App),
        motion: true,
        handlers: 1,
    },
    Lesson {
        title: "Scale and rotation",
        section: "Motion",
        description: "Click transformed buttons. Rust transforms paint and hit areas together.",
        id: "transforms",
        source: include_str!("../examples/tour/transforms.rs"),
        app: || crate::UiApp::new(transforms::App),
        motion: true,
        handlers: 2,
    },
    Lesson {
        title: "Enter and exit",
        section: "Motion",
        description: "Toggle a card or None. Rust retains the exiting presentation until its animation finishes.",
        id: "presence",
        source: include_str!("../examples/tour/presence.rs"),
        app: || crate::UiApp::new(presence::App),
        motion: true,
        handlers: 1,
    },
    Lesson {
        title: "Layout movement",
        section: "Motion",
        description: "Toggle alignment. A spring animates the box between its layout destinations.",
        id: "layout-motion",
        source: include_str!("../examples/tour/layout-motion.rs"),
        app: || crate::UiApp::new(layout_motion::App),
        motion: true,
        handlers: 1,
    },
    Lesson {
        title: "Staggered entrance",
        section: "Motion",
        description: "Toggle the group. The parent delays each child entrance; reduced motion also applies.",
        id: "stagger",
        source: include_str!("../examples/tour/stagger.rs"),
        app: || crate::UiApp::new(stagger::App),
        motion: true,
        handlers: 1,
    },
];
/// Scripted event and motion times from the original parity gate.
#[derive(Clone, Copy, Debug)]
pub enum Action {
    Frame(f64),
    Click(usize, f64),
}
pub fn script(handlers: &[usize], motion: bool) -> Vec<Action> {
    use Action::{Click, Frame};
    let mut actions = vec![Frame(0.), Frame(16.)];
    if let Some(&first) = handlers.first() {
        if motion {
            actions.extend([
                Click(first, 100.),
                Frame(125.),
                Frame(220.),
                Click(first, 220.),
            ]);
            actions.extend([225., 350., 600., 1200., 2000.].map(Frame));
            actions.push(Click(first, 2000.));
            actions.extend([2120., 2240., 2500., 3400., 4000.].map(Frame));
            for &handler in handlers.iter().skip(1) {
                let time = 5000. + handler as f64 * 2000.;
                actions.push(Click(handler, time));
                actions.extend([120., 350., 1000.].map(|delta| Frame(time + delta)));
            }
        } else {
            actions.extend(
                handlers
                    .iter()
                    .cycle()
                    .take(2 * handlers.len() + 2)
                    .enumerate()
                    .map(|(step, &handler)| Click(handler, 100. + step as f64 * 100.)),
            );
        }
    }
    actions.push(Frame(20000.));
    actions
}
/// Find an actual hittable point, respecting clipping and transformed shapes.
pub fn click_point(scene: &deka_native_ui::scene::Scene, handler: usize) -> Option<(f32, f32)> {
    scene
        .targets
        .iter()
        .find(|t| t.handler == handler)
        .and_then(|t| t.rect.intersection(t.clip))
        .and_then(|rect| {
            (0..10)
                .flat_map(|y| {
                    (0..10).map(move |x| {
                        (
                            rect.x + rect.width * (x as f32 + 0.5) / 10.,
                            rect.y + rect.height * (y as f32 + 0.5) / 10.,
                        )
                    })
                })
                .find(|&(x, y)| scene.hit(x, y).is_some_and(|hit| hit.handler == handler))
        })
}

pub fn handler_nodes(node: &deka_native_ir::Node) -> Vec<(String, usize)> {
    fn visit(node: &deka_native_ir::Node, output: &mut Vec<(String, usize)>) {
        if let Some(handler) = node.on_click {
            output.push((node.id.clone(), handler));
        }
        for child in &node.children {
            visit(child, output);
        }
    }
    let mut output = vec![];
    visit(node, &mut output);
    output
}

#[path = "../examples/showcase/button.rs"]
mod showcase_button;
#[path = "../examples/showcase/input.rs"]
mod showcase_input;
#[path = "../examples/showcase/list.rs"]
mod showcase_list;
#[path = "../examples/showcase/tabs.rs"]
mod showcase_tabs;

#[path = "../examples/showcase/badge.rs"]
mod showcase_badge;
#[path = "../examples/showcase/dialog.rs"]
mod showcase_dialog;
#[path = "../examples/showcase/menu.rs"]
mod showcase_menu;
#[path = "../examples/showcase/toast.rs"]
mod showcase_toast;

/// Component source and themed constructors use the same compiled pipeline as
/// /tour. They are separate from the 27 unchanged DekaScript parity lessons.
pub struct Showcase {
    pub id: &'static str,
    pub title: &'static str,
    pub path: &'static str,
    pub source: &'static str,
    pub app: fn(crate::components::Theme) -> crate::UiApp,
}
pub static SHOWCASES: &[Showcase] = &[
    Showcase {
        id: "button",
        title: "Button",
        path: "crates/deka_ui/examples/showcase/button.rs",
        source: include_str!("../examples/showcase/button.rs"),
        app: |theme| {
            crate::UiApp::new(
                || crate::view! { <showcase_button::App theme={crate::signal(theme)}/> },
            )
        },
    },
    Showcase {
        id: "input",
        title: "Input",
        path: "crates/deka_ui/examples/showcase/input.rs",
        source: include_str!("../examples/showcase/input.rs"),
        app: |theme| {
            crate::UiApp::new(
                || crate::view! { <showcase_input::App theme={crate::signal(theme)}/> },
            )
        },
    },
    Showcase {
        id: "list",
        title: "List",
        path: "crates/deka_ui/examples/showcase/list.rs",
        source: include_str!("../examples/showcase/list.rs"),
        app: |theme| {
            crate::UiApp::new(
                || crate::view! { <showcase_list::App theme={crate::signal(theme)}/> },
            )
        },
    },
    Showcase {
        id: "tabs",
        title: "Tabs",
        path: "crates/deka_ui/examples/showcase/tabs.rs",
        source: include_str!("../examples/showcase/tabs.rs"),
        app: |theme| {
            crate::UiApp::new(
                || crate::view! { <showcase_tabs::App theme={crate::signal(theme)}/> },
            )
        },
    },
    Showcase {
        id: "badge",
        title: "Badge",
        path: "crates/deka_ui/examples/showcase/badge.rs",
        source: include_str!("../examples/showcase/badge.rs"),
        app: |theme| {
            crate::UiApp::new(|| crate::view! {<showcase_badge::App theme={crate::signal(theme)}/>})
        },
    },
    Showcase {
        id: "toast",
        title: "Toast",
        path: "crates/deka_ui/examples/showcase/toast.rs",
        source: include_str!("../examples/showcase/toast.rs"),
        app: |theme| {
            crate::UiApp::new(|| crate::view! {<showcase_toast::App theme={crate::signal(theme)}/>})
        },
    },
    Showcase {
        id: "dialog",
        title: "Dialog",
        path: "crates/deka_ui/examples/showcase/dialog.rs",
        source: include_str!("../examples/showcase/dialog.rs"),
        app: |theme| {
            crate::UiApp::new(
                || crate::view! {<showcase_dialog::App theme={crate::signal(theme)}/>},
            )
        },
    },
    Showcase {
        id: "menu",
        title: "Menu",
        path: "crates/deka_ui/examples/showcase/menu.rs",
        source: include_str!("../examples/showcase/menu.rs"),
        app: |theme| {
            crate::UiApp::new(|| crate::view! {<showcase_menu::App theme={crate::signal(theme)}/>})
        },
    },
];
/// Resolve a lesson or a component showcase in either explicit theme.
pub fn app(id: &str) -> Option<crate::UiApp> {
    if let Some(lesson) = LESSONS.iter().find(|lesson| lesson.id == id) {
        return Some((lesson.app)());
    }
    for showcase in SHOWCASES {
        for (name, theme) in [
            ("light", crate::components::Theme::Light),
            ("dark", crate::components::Theme::Dark),
        ] {
            if id == format!("component-{}-{name}", showcase.id) {
                return Some((showcase.app)(theme));
            }
        }
    }
    None
}
