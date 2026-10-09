#![cfg(feature = "web")]
use deka_native_ui::Application;
use deka_ui::{components::*, prelude::*, web::BrowserApp};
use serde_json::Value;
use std::{cell::RefCell, rc::Rc};

fn frame(app: &mut BrowserApp) -> Value {
    serde_json::from_str(&app.frame_at(560., 480., 1., 200., true).unwrap()).unwrap()
}
fn semantics(app: &BrowserApp) -> Vec<Value> {
    serde_json::from_str(&app.semantic_nodes().unwrap()).unwrap()
}
fn id(app: &BrowserApp, name: &str) -> String {
    semantics(app)
        .into_iter()
        .find(|n| n["name"] == name)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .into()
}
fn click(app: &mut BrowserApp, name: &str) {
    let target = id(app, name);
    let scene = frame(app);
    let rect = &scene["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == target)
        .unwrap()["rect"];
    let x = rect["x"].as_f64().unwrap() + rect["width"].as_f64().unwrap() / 2.;
    let y = rect["y"].as_f64().unwrap() + rect["height"].as_f64().unwrap() / 2.;
    assert!(app.pointer(x as f32, y as f32));
}
fn contains(scene: &Value, text: &str) -> bool {
    scene["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["text"] == text)
}

#[test]
fn button_pointer_keyboard_focus_signal_and_disabled_ingress() {
    let mut state = None;
    let app = UiApp::new(|| {
        let count = signal(0);
        let disabled = signal(false);
        state = Some((count, disabled));
        view! {<view class="p-4 gap-3">
            <Button id="action" label="Add" disabled={disabled} on_press={Rc::new(move || {count.update(|n| *n += 1).unwrap();})}/>
            <Input label="Name" value={signal(String::new())}/>
            <p>"Count: {count}"</p>
        </view>}
    });
    let mut browser = BrowserApp::new(app);
    frame(&mut browser);
    click(&mut browser, "Add");
    assert!(contains(&frame(&mut browser), "Count: 1"));
    let button = id(&browser, "Add");
    assert_eq!(browser.focus(), Some(button.clone()));
    assert!(browser.key("Enter", false));
    assert!(browser.key(" ", false));
    assert_eq!(state.unwrap().0.get().unwrap(), 3);
    assert!(browser.key("Tab", false));
    assert_eq!(browser.focus(), Some(id(&browser, "Name")));
    assert!(browser.key("Tab", true));
    assert_eq!(browser.focus(), Some(button.clone()));
    state.unwrap().1.set(true);
    assert!(!browser.activate(&button));
    assert_eq!(state.unwrap().0.get().unwrap(), 3);
    browser.blur();
    assert!(browser.key("Tab", false));
    assert_eq!(browser.focus(), Some(id(&browser, "Name")));
}

#[test]
fn input_pointer_two_way_signal_programmatic_updates_and_focus_order() {
    let mut value = None;
    let app = UiApp::new(|| {
        let text = signal("start".to_owned());
        value = Some(text);
        view! {<view class="p-4 gap-3">
            <Input id="editor" label="Name" value={text}/>
            <Button label="Clear" on_press={Rc::new(move ||text.set(String::new()))}/>
            <p>"Hello {text}"</p>
        </view>}
    });
    let input_node = app.tree().get_element_by_id("editor").unwrap();
    let mut browser = BrowserApp::new(app);
    let scene = frame(&mut browser);
    let editor = id(&browser, "Name");
    let node = scene["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == editor)
        .unwrap();
    let rect = &node["rect"];
    browser.pointer(
        rect["x"].as_f64().unwrap() as f32 + 8.,
        rect["y"].as_f64().unwrap() as f32 + 8.,
    );
    assert_eq!(browser.focus(), Some(editor.clone()));
    assert!(browser.input(&editor, "日本"));
    assert_eq!(value.unwrap().get().unwrap(), "日本");
    assert!(contains(&frame(&mut browser), "Hello 日本"));
    value.unwrap().set("Sami".into());
    assert_eq!(
        semantics(&browser)
            .into_iter()
            .find(|n| n["id"] == editor)
            .unwrap()["value"],
        "Sami"
    );
    browser.blur();
    input_node.focus().unwrap();
    frame(&mut browser);
    assert_eq!(browser.focus(), Some(editor.clone()));
    assert_eq!(browser.take_requested_focus(), Some(editor.clone()));
    assert_eq!(
        browser.take_requested_focus(),
        None,
        "deliver focus only once"
    );
    browser.blur();
    assert!(browser.key("Tab", false));
    assert_eq!(browser.focus(), Some(editor));
    assert!(browser.key("Tab", false));
    assert_eq!(browser.focus(), Some(id(&browser, "Clear")));
    assert!(browser.key("Enter", false));
    assert_eq!(value.unwrap().get().unwrap(), "");
}

#[test]
fn list_reorder_keeps_identity_focus_handlers_and_updates_labels() {
    let mut state = None;
    let app = UiApp::new(|| {
        let items = signal(vec![
            ListItem::new("a", "Alpha"),
            ListItem::new("b", "Beta"),
        ]);
        let selected = signal(None);
        state = Some((items, selected));
        view! {<List id="list" label="Projects" items={items} selected={selected}/>}
    });
    let retained = app.tree();
    let alpha = retained.query_all("button").unwrap()[0].clone();
    let beta = retained.query_all("button").unwrap()[1].clone();
    let mut browser = BrowserApp::new(app);
    frame(&mut browser);
    let beta_id = id(&browser, "Beta");
    click(&mut browser, "Beta");
    assert_eq!(state.unwrap().1.get().unwrap().as_deref(), Some("b"));
    state
        .unwrap()
        .0
        .update(|items| {
            items.reverse();
            items[0].label = "Renamed".into();
        })
        .unwrap();
    frame(&mut browser);
    assert_eq!(
        retained.query_all("button").unwrap(),
        [beta.clone(), alpha.clone()]
    );
    assert_eq!(id(&browser, "Renamed"), beta_id);
    assert_eq!(browser.focus(), Some(beta_id.clone()));
    assert!(browser.key("Tab", false));
    assert_eq!(browser.focus(), Some(id(&browser, "Alpha")));
    assert!(browser.key("Enter", false));
    assert_eq!(state.unwrap().1.get().unwrap().as_deref(), Some("a"));
    state
        .unwrap()
        .0
        .update(|items| {
            items.remove(1);
            items.push(ListItem::new("c", "Gamma"));
        })
        .unwrap();
    frame(&mut browser);
    assert!(alpha.parent_node().is_none());
    assert_eq!(browser.focus(), None, "removed row clears focus");
    assert_eq!(retained.query_all("button").unwrap()[0], beta);
    browser.blur();
    assert!(browser.key("Tab", false));
    assert!(browser.key(" ", false));
    assert_eq!(state.unwrap().1.get().unwrap().as_deref(), Some("b"));
    assert_eq!(
        semantics(&browser)
            .iter()
            .filter(|n| n["role"] == "listitem")
            .count(),
        2
    );
}

#[test]
fn keyed_rows_construct_once_keep_local_state_and_dispose_removed_owners() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let saved = Rc::new(RefCell::new(Vec::new()));
    let mut order = None;
    let app = UiApp::new(|| {
        let keys = signal(vec!["a".to_owned(), "b".to_owned()]);
        order = Some(keys);
        View::keyed(move || keys.get().unwrap(), {
            let calls = calls.clone();
            let saved = saved.clone();
            move |key| {
                calls.borrow_mut().push(key.clone());
                let clicks = signal(0);
                saved.borrow_mut().push(clicks);
                view! {<button onClick={move |_| { clicks.update(|n| *n += 1).unwrap(); }}>"{key}: {clicks}"</button>}
            }
        })
    });
    let buttons = app.tree().query_all("button").unwrap();
    let first_id = app
        .semantics()
        .into_iter()
        .find(|n| n.name == "a: 0")
        .unwrap()
        .id;
    let first_token = app.tree().children[0].children[0].on_click.unwrap();
    assert!(app.dispatch(first_token));
    assert_eq!(saved.borrow()[0].get().unwrap(), 1);
    order.unwrap().set(vec!["b".into(), "a".into()]);
    assert_eq!(&*calls.borrow(), &["a", "b"]);
    assert_eq!(
        app.tree().query_all("button").unwrap(),
        [buttons[1].clone(), buttons[0].clone()]
    );
    order.unwrap().set(vec!["b".into()]);
    assert!(
        saved.borrow()[0].get().is_err(),
        "removed row disposes its signal"
    );
    assert!(
        !app.dispatch_to(&first_id, Event::Click),
        "removed handler is not routed to another row"
    );
    order.unwrap().set(vec!["b".into(), "a".into()]);
    assert_eq!(&*calls.borrow(), &["a", "b", "a"]);
    assert_ne!(app.tree().query_all("button").unwrap()[1], buttons[0]);
    assert_eq!(saved.borrow()[2].get().unwrap(), 0);
    drop(app);
    assert!(saved.borrow()[1].get().is_err());
    assert!(saved.borrow()[2].get().is_err());
}

#[test]
fn duplicate_key_edit_reports_error_and_preserves_mounted_rows() {
    let mut keys = None;
    let app = UiApp::new_with_error_sink(
        || {
            let order = signal(vec!["a".to_owned()]);
            keys = Some(order);
            View::keyed(move || order.get().unwrap(), |key| view! {<p>"{key}"</p>})
        },
        Some(Rc::new(|_| {})),
    );
    let before = app.tree().query("p").unwrap().unwrap();
    keys.unwrap().set(vec!["a".into(), "a".into()]);
    assert_eq!(app.take_errors(), ["duplicate list key"]);
    assert_eq!(app.tree().query_all("p").unwrap(), [before]);
    keys.unwrap().set(vec!["a".into(), "b".into()]);
    assert_eq!(app.tree().query_all("p").unwrap().len(), 2);
}

#[test]
fn tabs_pointer_roving_keyboard_wrap_disabled_skipping_panels_and_signals() {
    let mut selected = None;
    let app = UiApp::new(|| {
        let choice = signal("a".to_owned());
        selected = Some(choice);
        let mut disabled = TabItem::new("b", "Disabled", || View::text("Never"));
        disabled.disabled = true;
        let tabs = vec![
            TabItem::new("a", "Alpha", || View::text("Alpha panel")),
            disabled,
            TabItem::new("c", "Gamma", || View::text("Gamma panel")),
        ];
        view! {<view class="p-4 gap-3">
            <Tabs id="tabs" label="Workspace" items={tabs} selected={choice}/>
            <Input label="Next field" value={signal(String::new())}/>
        </view>}
    });
    let mut browser = BrowserApp::new(app);
    assert!(contains(&frame(&mut browser), "Alpha panel"));
    let alpha = id(&browser, "Alpha");
    // Tab/panel share their accessible names; pick by role.
    let gamma = semantics(&browser)
        .into_iter()
        .find(|n| n["name"] == "Gamma" && n["role"] == "tab")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(browser.key("Tab", false));
    assert_eq!(browser.focus(), Some(alpha.clone()));
    assert!(browser.key("ArrowRight", false));
    assert_eq!(selected.unwrap().get().unwrap(), "c");
    assert_eq!(browser.focus(), Some(gamma.clone()));
    let next = frame(&mut browser);
    assert!(contains(&next, "Gamma panel"));
    assert!(!contains(&next, "Alpha panel"));
    assert!(browser.key("ArrowRight", false));
    assert_eq!(browser.focus(), Some(alpha.clone()));
    assert!(browser.key("ArrowLeft", false));
    assert_eq!(browser.focus(), Some(gamma.clone()));
    assert!(browser.key("Home", false));
    assert_eq!(browser.focus(), Some(alpha.clone()));
    assert!(browser.key("End", false));
    assert_eq!(browser.focus(), Some(gamma.clone()));
    assert!(browser.key("Tab", false));
    assert_eq!(browser.focus(), Some(id(&browser, "Next field")));
    assert!(browser.key("Tab", true));
    assert_eq!(browser.focus(), Some(gamma.clone()));
    click(&mut browser, "Alpha");
    assert_eq!(selected.unwrap().get().unwrap(), "a");
    assert_eq!(browser.focus(), Some(alpha));
    let nodes = semantics(&browser);
    assert_eq!(
        nodes
            .iter()
            .filter(|n| n["role"] == "tab" && n["selected"] == true && n["tabIndex"] == 0)
            .count(),
        1
    );
    assert!(
        nodes
            .iter()
            .filter(|n| n["role"] == "tab")
            .all(|n| n["controls"].is_string())
    );
    selected.unwrap().set("c".into());
    assert!(contains(&frame(&mut browser), "Gamma panel"));
}

#[test]
fn live_theme_signal_changes_all_four_components_without_replacing_controls() {
    let mut theme = None;
    let app = UiApp::new(|| {
        let current = signal(Theme::Light);
        theme = Some(current);
        view! {<view>
            <Button label="Action" theme={current} on_press={Rc::new(||{})}/>
            <Input label="Name" theme={current} value={signal(String::new())}/>
            <List label="Projects" theme={current} selected={signal(None)} items={signal(vec![ListItem::new("a","Alpha")])}/>
            <Tabs id="tabs" label="Workspace" theme={current} selected={signal("a".into())} items={vec![TabItem::new("a","Overview",||View::text("Panel"))]}/>
        </view>}
    });
    let buttons = app.tree().query_all("button").unwrap();
    let before = app.tree().snapshot();
    theme.unwrap().set(Theme::Dark);
    assert_ne!(app.tree().snapshot(), before);
    assert_eq!(app.tree().query_all("button").unwrap(), buttons);
    assert!(app.take_errors().is_empty());
}

#[test]
fn enter_exit_and_repeat_press_present_motion_and_reduced_motion_snaps() {
    use deka_native_ui::animation::Animator;
    let mut visible = None;
    let app = UiApp::new(|| {
        let shown = signal(true);
        visible = Some(shown);
        view! {<view>
            <Button label="Press" on_press={Rc::new(||{})}/>
            {move || shown.get().unwrap().then(|| View::element("view").attr("class",format!("{} {}",MotionPreset::Enter.classes(),MotionPreset::Exit.classes())).child("Presence"))}
        </view>}
    });
    let mut normal = Animator::default();
    let mut reduced = Animator::default();
    let initial = app.tree().snapshot();
    let (enter, moving) = normal.sample(&initial, 0., false);
    assert!(moving);
    assert_eq!(enter.children[1].style.opacity, 0.);
    let (snap, moving) = reduced.sample(&initial, 0., true);
    assert!(!moving);
    assert_eq!(snap.children[1].style.opacity, 1.);
    assert!(
        normal.sample(&initial, 80., false).0.children[1]
            .style
            .opacity
            > 0.
    );
    assert!(!normal.sample(&initial, 200., false).1);
    let handler = initial.children[0].on_click.unwrap();
    assert!(app.dispatch(handler));
    let pressed = app.tree().snapshot();
    normal.sample(&pressed, 200., false);
    let (press, moving) = normal.sample(&pressed, 248., false);
    assert!(moving);
    assert!(press.children[0].style.scale < 1.);
    assert_eq!(
        reduced.sample(&pressed, 248., true).0.children[0]
            .style
            .scale,
        1.
    );
    assert!(app.dispatch(handler));
    let again = app.tree().snapshot();
    normal.sample(&again, 248., false);
    assert!(
        normal.sample(&again, 288., false).1,
        "second press restarts the timeline"
    );
    assert!(!normal.sample(&again, 400., false).1);
    visible.unwrap().set(false);
    let removed = app.tree().snapshot();
    let (exit, moving) = normal.sample(&removed, 400., false);
    assert!(moving);
    assert_eq!(exit.children.len(), 2, "exit keeps an inert visual ghost");
    assert!(exit.children[1].on_click.is_none());
    assert_eq!(normal.sample(&removed, 600., false).0.children.len(), 1);
    assert_eq!(reduced.sample(&removed, 400., true).0.children.len(), 1);
}
