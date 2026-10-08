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

#[test]
fn incremental_updates_only_send_changed_nodes_and_parent_child_removals() {
    let mut text_handle = None;
    let mut shown_handle = None;
    let mut session = DesktopSession::new(UiApp::new(|| {
        let text = signal("old".to_owned());
        let shown = signal(true);
        text_handle = Some(text);
        shown_handle = Some(shown);
        view! { <view>
            <input aria-label="Name" value={text}/>
            {move || if shown.get().unwrap() {view!{<button>"Temporary"</button>}} else {View::fragment([])}}
            <button>"Stable"</button>
        </view> }
    }));
    session.frame(400., 300., 1.);
    let first = session.accessibility(1.);
    assert!(first.tree.is_some());
    let input = first
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Name"))
        .unwrap()
        .0;
    let stable = first
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Stable"))
        .unwrap()
        .0;
    let temporary = first
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Temporary"))
        .unwrap()
        .0;
    let idle = session.accessibility(1.);
    assert!(idle.tree.is_none());
    assert!(
        idle.nodes.is_empty(),
        "unchanged text runs must retain their identities too"
    );
    text_handle.unwrap().set("new".into());
    session.frame(400., 300., 1.);
    let changed = session.accessibility(1.);
    assert!(changed.tree.is_none());
    assert_eq!(
        changed
            .nodes
            .iter()
            .find(|(id, _)| *id == input)
            .unwrap()
            .1
            .value(),
        Some("new")
    );
    assert!(!changed.nodes.iter().any(|(id, _)| *id == stable));
    shown_handle.unwrap().set(false);
    session.frame(400., 300., 1.);
    let removed = session.accessibility(1.);
    assert!(removed.tree.is_none());
    assert!(!removed.nodes.iter().any(|(id, _)| *id == temporary));
    let parent = first
        .nodes
        .iter()
        .find(|(_, n)| n.children().contains(&temporary))
        .unwrap()
        .0;
    assert!(
        !removed
            .nodes
            .iter()
            .find(|(id, _)| *id == parent)
            .unwrap()
            .1
            .children()
            .contains(&temporary),
        "parent update must remove the old node from the AT tree"
    );
}

#[test]
fn queued_winit_click_and_focus_reject_newly_hidden_or_disabled_parent() {
    for attribute in ["aria-hidden", "disabled"] {
        let clicks = Rc::new(Cell::new(0));
        let output = clicks.clone();
        let mut session = DesktopSession::new(UiApp::new(move || {
            view! {
                <view id="parent"><button aria-label="Child" onClick={move |_|output.set(output.get()+1)}>"Child"</button></view>
            }
        }));
        session.frame(400., 300., 1.);
        let first = session.accessibility(1.);
        let child = first
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Child"))
            .unwrap()
            .0;
        assert!(session.accessibility_event(&action(Action::Click, child, None)));
        assert_eq!(clicks.get(), 1);
        session
            .app()
            .tree()
            .get_element_by_id("parent")
            .unwrap()
            .set_attribute(attribute, "true")
            .unwrap();
        // Queued payloads arrive before the next frame/projection refresh.
        assert!(!session.accessibility_event(&action(Action::Click, child, None)));
        assert!(!session.accessibility_event(&action(Action::Focus, child, None)));
        assert_eq!(clicks.get(), 1);
        assert_eq!(session.focus(), None);
    }
}
