#![cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
use deka_native_ui::{
    Application,
    window::{DesktopSession, KeyInput},
};
use deka_ui::prelude::*;
use std::{cell::Cell, rc::Rc};
// Requests use the adapter's public event type, handled by the same Content
// ingress as winit's user_event callback; no application handler is called here.
use deka_native_ui::window::accesskit_events::{
    Action, ActionData, ActionRequest, NodeId, Role, TreeId, WindowEvent,
};
fn action(action: Action, target: NodeId, data: Option<ActionData>) -> WindowEvent {
    WindowEvent::ActionRequested(ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data,
    })
}
fn key(shift: bool) -> KeyInput {
    KeyInput {
        name: "tab".into(),
        down: true,
        shift,
        ..Default::default()
    }
}
#[test]
fn adapter_actions_change_rendered_signal_value_and_focus() {
    let count = Rc::new(Cell::new(0));
    let output = count.clone();
    let mut session = DesktopSession::new(UiApp::new(move || {
        let text = signal("start".to_owned());
        view! { <view className="p-4 gap-4">
            <input id="editor" aria-label="Name" value={text}/>
            <button id="button" onClick={move |_| output.set(output.get()+1)}>"Save"</button>
            <p>"Typed: {text}"</p>
        </view> }
    }));
    session.frame(400., 300., 2.);
    let tree = session.accessibility(2.);
    let input = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Name"))
        .unwrap()
        .0;
    let button = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Save"))
        .unwrap()
        .0;
    assert!(session.accessibility_event(&action(Action::Focus, input, None)));
    assert_eq!(session.accessibility(2.).focus, input);
    assert!(session.accessibility_event(&action(
        Action::SetValue,
        input,
        Some(ActionData::Value("日本".into()))
    )));
    assert!(
        session
            .frame(400., 300., 2.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Typed: 日本"))
    );
    let updated = session.accessibility(2.);
    let node = &updated.nodes.iter().find(|(id, _)| *id == input).unwrap().1;
    assert_eq!(node.role(), Role::TextInput);
    assert_eq!(node.value(), Some("日本"));
    assert!(node.text_selection().is_some());
    assert!(!node.children().is_empty());
    let mut selection = *node.text_selection().unwrap();
    selection.anchor.character_index = 0;
    assert!(session.accessibility_event(&action(
        Action::SetTextSelection,
        input,
        Some(ActionData::SetTextSelection(selection))
    )));
    assert!(session.event(
        &winit::event::WindowEvent::Ime(winit::event::Ime::Commit("置換".into())),
        2.
    ));
    assert!(
        session
            .frame(400., 300., 2.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Typed: 置換"))
    );
    assert!(session.accessibility_event(&action(Action::Click, button, None)));
    assert_eq!(count.get(), 1);
    assert!(!session.accessibility_event(&action(Action::Click, NodeId(999999), None)));
}
#[test]
fn tab_order_skips_disabled_hidden_and_negative_but_explicit_focus_works() {
    let mut session = DesktopSession::new(UiApp::new(|| {
        view! {
            <view>
                <button id="natural">"Natural"</button>
                <input id="second" tabIndex=2 aria-label="Second"/>
                <input id="first" tabIndex=1 aria-label="First"/>
                <input id="disabled" disabled=true aria-label="Disabled"/>
                <view aria-hidden=true><input id="hidden" aria-label="Hidden"/></view>
                <input id="explicit" tabIndex=-1 aria-label="Explicit"/>
            </view>
        }
    }));
    session.frame(400., 300., 1.);
    let semantics = session.app().semantics();
    let order = deka_native_ui::tab_order(&semantics);
    assert_eq!(
        order,
        ["First", "Second", "Natural"].map(|name| semantics
            .iter()
            .find(|n| n.name == name)
            .unwrap()
            .id
            .clone())
    );
    for target in &order {
        session.keyboard(key(false));
        assert_eq!(session.focus(), Some(target.as_str()));
    }
    session.keyboard(key(false));
    assert_eq!(session.focus(), None);
    session.keyboard(key(true));
    assert_eq!(session.focus(), order.last().map(String::as_str));
    let tree = session.accessibility(1.);
    assert!(!tree.nodes.iter().any(|(_, n)| n.label() == Some("Hidden")));
    let explicit = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Explicit"))
        .unwrap()
        .0;
    let disabled = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Disabled"))
        .unwrap()
        .0;
    assert!(session.accessibility_event(&action(Action::Focus, explicit, None)));
    assert!(!session.accessibility_event(&action(Action::Focus, disabled, None)));
}

#[test]
fn removed_control_loses_adapter_route_and_identity_is_not_reused() {
    let mut session = DesktopSession::new(UiApp::new(|| {
        let visible = signal(true);
        view! { <view>
            {move || if visible.get().unwrap() { view! { <input aria-label="Temporary"/> } } else { View::fragment([]) }}
            <button onClick={move |_|visible.set(!visible.get().unwrap())}>"Toggle"</button>
        </view> }
    }));
    session.frame(400., 300., 1.);
    let tree = session.accessibility(1.);
    let temporary = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Temporary"))
        .unwrap()
        .0;
    let toggle = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Toggle"))
        .unwrap()
        .0;
    assert!(session.accessibility_event(&action(Action::Click, toggle, None)));
    session.frame(400., 300., 1.);
    assert!(
        !session
            .accessibility(1.)
            .nodes
            .iter()
            .any(|(id, _)| *id == temporary)
    );
    assert!(!session.accessibility_event(&action(
        Action::SetValue,
        temporary,
        Some(ActionData::Value("stale".into()))
    )));
    assert!(session.accessibility_event(&action(Action::Click, toggle, None)));
    session.frame(400., 300., 1.);
    let next = session.accessibility(1.);
    assert_ne!(
        next.nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Temporary"))
            .unwrap()
            .0,
        temporary
    );
}
