use deka_ui::prelude::*;

mod widgets {
    use super::*;
    #[component]
    pub fn Card(title: String, #[prop(default = 2)] copies: usize, children: Children) -> View {
        view! { <div class="p-4">{(0..copies).map(move |_| view! {<p>"{title}"</p>})}{children()}</div> }
    }
}
use widgets::Card;

#[component]
fn App() -> View {
    let mut count = signal(0);
    view! {
        <view>
            <Card title="Counter"><span>"Count: {count}"</span></Card>
            <button onClick={move |_| count += 1}>"Add one"</button>
        </view>
    }
}

fn text(node: &deka_native_ir::Node) -> String {
    node.text.clone().unwrap_or_default() + &node.children.iter().map(text).collect::<String>()
}

#[test]
fn imported_component_props_defaults_children_and_interpolation_run_on_the_real_host() {
    let app = UiApp::new(App);
    let before = app.tree();
    assert_eq!(text(&before), "CounterCounterCount: 0Add one");
    app.dispatch(0);
    let after = app.tree();
    assert_eq!(text(&after), "CounterCounterCount: 1Add one");
    assert_eq!(
        before.children[0].children[..2],
        after.children[0].children[..2]
    );
    assert_eq!(before.children[1], after.children[1]);
    assert_eq!(app.property_patches(), 1);
}

#[test]
fn explicit_default_override_and_absent_children_use_typed_props() {
    let app = UiApp::new(|| view! {<view><Card title="Once" copies={1}/></view>});
    assert_eq!(text(&app.tree()), "Once");
    let direct = UiApp::new(|| {
        Card(widgets::CardProps {
            title: "Direct".into(),
            copies: 1,
            children: Box::new(|| view! {<p>" child"</p>}),
        })
    });
    assert_eq!(text(&direct.tree()), "Direct child");
}

#[test]
fn class_colon_toggle_live_attributes_option_and_iterator_children_patch() {
    let app = UiApp::new(|| {
        let open = signal(false);
        view! {
            <view>
                <p class={move || if open.get().unwrap(){"p-4"}else{"p-2"}} class:rounded={open}>"Kept"</p>
                {move || open.get().unwrap().then(|| view!{<p>"Optional"</p>})}
                {(0..2).map(|n| view!{<p>{n}</p>})}
                <button onClick={move |_| open.toggle()}>"Toggle"</button>
            </view>
        }
    });
    let before = app.tree();
    app.dispatch(0);
    let after = app.tree();
    assert_eq!(after.children[0].style.padding.top, 16.);
    assert_eq!(after.children[0].style.radius, 4.);
    assert_eq!(after.children[0].children, before.children[0].children);
    assert_eq!(after.children[2..], before.children[1..]);
    assert_eq!(text(&after), "KeptOptional01Toggle");
}

#[test]
fn escaped_braces_and_repeated_string_interpolations_preserve_text_fragments() {
    let app = UiApp::new(|| {
        let name = String::from("Deka");
        view! {<view><p>"{{{name}}}"</p><p>"{name}"</p></view>}
    });
    assert_eq!(text(&app.tree()), "{Deka}Deka");
    assert_eq!(app.tree().children[0].children.len(), 3);
}

#[test]
fn input_and_keydown_markup_register_real_typed_closures() {
    let app = UiApp::new(|| {
        let value = signal(String::new());
        view! {<view><input value={move || value.get().unwrap()}
        onInput={move |event| {if let Event::Input(text)=event {value.set(text);}}}
        onKeyDown={move |event| {if event==Event::KeyDown("Escape".into()) {value.set(String::new());}}}/>{value}</view>}
    });
    let id = app.tree().children[0].id.clone();
    app.dispatch_to(&id, Event::Input("Typed".into()));
    assert_eq!(text(&app.tree()), "Typed");
    app.dispatch_to(&id, Event::KeyDown("Escape".into()));
    assert_eq!(text(&app.tree()), "");
}

#[component]
fn Defaults(#[prop(default)] count: usize, #[prop(default = 7)] step: usize) -> View {
    view! {<view>{count}{step}</view>}
}
#[test]
fn default_props_support_launch_style_functions_and_root_expression_fragments() {
    assert_eq!(text(&UiApp::new(Defaults).tree()), "07");
    let app = UiApp::new(|| view! {{Some(view!{<p>"Root option"</p>})}});
    assert_eq!(text(&app.tree()), "Root option");
}

#[test]
fn static_expression_and_live_classes_style_the_same_retained_attribute() {
    let app = UiApp::new(|| {
        let open = signal(false);
        let classes = "p-3";
        view! {<view>
            <p id="static" class="p-4">"Static"</p>
            <p id="expression" class={classes}>"Expression"</p>
            <p id="live" class={move || if open.get().unwrap() {"p-6"} else {"p-2"}}
                class:rounded={open}>"Live"</p>
            <button onClick={move |_| open.toggle()}>"Toggle"</button>
        </view>}
    });
    let static_node = app.tree().get_element_by_id("static").unwrap();
    let expression = app.tree().get_element_by_id("expression").unwrap();
    let live = app.tree().get_element_by_id("live").unwrap();
    assert_eq!(static_node.get_attribute("class").as_deref(), Some("p-4"));
    assert_eq!(expression.get_attribute("class").as_deref(), Some("p-3"));
    assert_eq!(live.get_attribute("class").as_deref(), Some("p-2"));
    assert_eq!(app.tree().children[0].style.padding.top, 16.);
    assert_eq!(app.tree().children[1].style.padding.top, 12.);
    assert_eq!(app.tree().children[2].style.padding.top, 8.);
    app.dispatch(0);
    assert_eq!(app.tree().get_element_by_id("live").unwrap(), live);
    assert_eq!(live.get_attribute("class").as_deref(), Some("p-6 rounded"));
    assert_eq!(live.class_list().tokens(), ["p-6", "rounded"]);
    assert_eq!(app.tree().children[2].style.padding.top, 24.);
    assert_eq!(app.tree().children[2].style.radius, 4.);
    live.class_list().add("opacity-25").unwrap();
    assert_eq!(
        live.get_attribute("class").as_deref(),
        Some("p-6 rounded opacity-25")
    );
    assert_eq!(app.tree().children[2].style.opacity, 0.25);
    app.dispatch(0);
    assert_eq!(live.get_attribute("class").as_deref(), Some("p-2"));
    assert_eq!(app.tree().children[2].style.padding.top, 8.);
    assert_eq!(app.tree().children[2].style.opacity, 1.);
    assert!(app.take_errors().is_empty());
}
