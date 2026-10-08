#![cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
use deka_native_ui::window::{DesktopSession, KeyInput, TextClipboard};
use deka_ui::prelude::*;
use std::{cell::RefCell, rc::Rc};
use winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, Ime, MouseButton, WindowEvent},
};
fn app(value: &str, multiline: bool) -> DesktopSession<UiApp> {
    let value = value.to_owned();
    let app = UiApp::new(move || {
        let text = signal(value);
        let tag = if multiline { "textarea" } else { "input" };
        View::element("view")
            .attr("class", "p-4 gap-4")
            .child(View::element(tag).attr("id", "editor").value(text))
            .child(view! { <p>"Value: {text}"</p> })
    });
    let mut session = DesktopSession::new(app);
    session.frame(400., 300., 1.);
    session.keyboard(key("tab"));
    session
}
fn key(name: &str) -> KeyInput {
    KeyInput {
        name: name.into(),
        down: true,
        ..Default::default()
    }
}
fn command(name: &str) -> KeyInput {
    KeyInput {
        command: true,
        ..key(name)
    }
}
fn value(session: &DesktopSession<UiApp>) -> String {
    session
        .app()
        .tree()
        .get_element_by_id("editor")
        .unwrap()
        .get_attribute("value")
        .unwrap()
}
fn commit(session: &mut DesktopSession<UiApp>, value: &str) {
    assert!(session.event(&WindowEvent::Ime(Ime::Commit(value.into())), 1.));
}
#[test]
fn os_ime_preedit_commit_cancel_and_dead_keys_update_signal_only_on_commit() {
    let mut session = app("Hello ", false);
    session.keyboard(command("right"));
    let before = session.frame(400., 300., 2.).clone();
    assert!(session.event(
        &WindowEvent::Ime(Ime::Preedit("にほん".into(), Some((9, 9)))),
        2.
    ));
    assert_eq!(value(&session), "Hello ");
    let composing = session.frame(400., 300., 2.).clone();
    assert_ne!(
        serde_json::to_string(&before.paint).unwrap(),
        serde_json::to_string(&composing.paint).unwrap()
    );
    assert!(
        composing
            .paint
            .iter()
            .any(|p| p.image.is_none() && p.rect.height == 1.)
    );
    assert!(session.ime_area().unwrap().width > 0.);
    commit(&mut session, "日本");
    assert_eq!(value(&session), "Hello 日本");
    assert!(
        session
            .frame(400., 300., 1.)
            .nodes
            .iter()
            .any(|node| node.text.as_deref() == Some("Value: Hello 日本"))
    );
    assert!(session.event(&WindowEvent::Ime(Ime::Preedit("語".into(), None)), 1.));
    session.event(&WindowEvent::Ime(Ime::Disabled), 1.);
    assert_eq!(value(&session), "Hello 日本");
    session.keyboard(command("a"));
    session.event(
        &WindowEvent::Ime(Ime::Preedit("´".into(), Some((2, 2)))),
        1.,
    );
    commit(&mut session, "é");
    assert_eq!(value(&session), "é");
    session.keyboard(command("z"));
    assert_eq!(value(&session), "Hello 日本");
    session.keyboard(KeyInput {
        shift: true,
        ..command("z")
    });
    assert_eq!(value(&session), "é");
    session.keyboard(command("a"));
    session.event(
        &WindowEvent::Ime(Ime::Preedit("取消".into(), Some((6, 6)))),
        1.,
    );
    session.event(&WindowEvent::Focused(false), 1.);
    assert_eq!(value(&session), "é");
    commit(&mut session, "a");
    assert_eq!(
        value(&session),
        "a",
        "cancel restores the pre-composition selection"
    );
}
#[test]
fn shaped_selection_word_line_jumps_and_undo_use_window_key_ingress() {
    let mut session = app("one two\nthree", true);
    session.keyboard(command("down"));
    session.keyboard(KeyInput {
        shift: true,
        word: true,
        ..key("left")
    });
    commit(&mut session, "四");
    assert_eq!(value(&session), "one two\n四");
    session.keyboard(command("z"));
    assert_eq!(value(&session), "one two\nthree");
    session.keyboard(command("up"));
    session.keyboard(KeyInput {
        shift: true,
        ..command("right")
    });
    commit(&mut session, "first");
    assert_eq!(value(&session), "first\nthree");
    session.keyboard(command("down"));
    session.keyboard(key("enter"));
    commit(&mut session, "last");
    assert_eq!(value(&session), "first\nthree\nlast");
    let mut emoji = app("a👨‍👩‍👧‍👦b", false);
    emoji.keyboard(command("right"));
    emoji.keyboard(key("left"));
    emoji.keyboard(key("delete"));
    assert_eq!(value(&emoji), "a👨‍👩‍👧‍👦");
    emoji.keyboard(key("backspace"));
    assert_eq!(value(&emoji), "a", "backspace removes the emoji cluster");
}
struct Clipboard(Rc<RefCell<String>>);
impl TextClipboard for Clipboard {
    fn get(&mut self) -> Result<String, String> {
        Ok(self.0.borrow().clone())
    }
    fn set(&mut self, s: &str) -> Result<(), String> {
        *self.0.borrow_mut() = s.into();
        Ok(())
    }
}
#[test]
fn os_mouse_drag_and_clipboard_shortcuts_edit_the_bound_retained_tree() {
    let mut session = app("selected", false);
    let clipboard = Rc::new(RefCell::new(String::new()));
    session.clipboard(Clipboard(clipboard.clone()));
    let id = session.focus().unwrap().to_owned();
    let rect = session
        .frame(400., 300., 1.)
        .nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap()
        .rect;
    let mouse = |state| WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    };
    let point = |x| WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x as f64, (rect.y + 8.) as f64),
    };
    session.event(&point(rect.x + 4.), 1.);
    session.event(&mouse(ElementState::Pressed), 1.);
    session.event(&point(rect.x + 200.), 1.);
    session.event(&mouse(ElementState::Released), 1.);
    session.keyboard(command("c"));
    assert_eq!(&*clipboard.borrow(), "selected");
    session.keyboard(command("x"));
    assert_eq!(value(&session), "");
    *clipboard.borrow_mut() = "東京\ntext".into();
    session.keyboard(command("v"));
    assert_eq!(value(&session), "東京text", "input excludes line breaks");
    session.keyboard(command("z"));
    assert_eq!(value(&session), "");
    session.keyboard(command("z"));
    assert_eq!(value(&session), "selected");
}
#[test]
fn external_signal_changes_replace_the_editor_without_stale_undo() {
    let mut signal_handle = None;
    let app = UiApp::new(|| {
        let text = signal("before".to_owned());
        signal_handle = Some(text);
        view! {<input id="editor" value={text}/>}
    });
    let mut session = DesktopSession::new(app);
    session.frame(400., 300., 1.);
    session.keyboard(key("tab"));
    session.keyboard(command("a"));
    commit(&mut session, "edited");
    signal_handle.unwrap().set("external".to_owned());
    session.frame(400., 300., 1.);
    session.keyboard(command("z"));
    assert_eq!(value(&session), "external");
    session.keyboard(command("a"));
    commit(&mut session, "new");
    assert_eq!(value(&session), "new");
}

#[test]
fn one_way_initial_value_survives_frames_and_two_way_updates_before_user_handler() {
    let recorded = Rc::new(RefCell::new(String::new()));
    let output = recorded.clone();
    let app = UiApp::new(move || {
        let text = signal(String::new());
        view! {<input id="editor" value={text} onInput={move |_|*output.borrow_mut()=text.get().unwrap()}/>}
    });
    let mut session = DesktopSession::new(app);
    session.frame(400., 300., 1.);
    session.keyboard(key("tab"));
    commit(&mut session, "ordered");
    assert_eq!(&*recorded.borrow(), "ordered");
    let mut session =
        DesktopSession::new(UiApp::new(|| view! {<input id="editor" value="initial"/>}));
    session.frame(400., 300., 1.);
    session.keyboard(key("tab"));
    session.keyboard(command("a"));
    commit(&mut session, "typed");
    let before = serde_json::to_string(&session.frame(400., 300., 1.).paint).unwrap();
    assert_eq!(
        before,
        serde_json::to_string(&session.frame(400., 300., 1.).paint).unwrap()
    );
    assert_eq!(
        value(&session),
        "typed",
        "edits write the retained effective value"
    );
}

#[test]
fn two_way_handler_can_reject_an_edit_to_the_previous_value() {
    let mut session = DesktopSession::new(UiApp::new(|| {
        let text = signal("accepted".to_owned());
        view! {
            <view><input id="editor" value={text} onInput={move |_| text.set("accepted".into())}/>
            <p>"Value: {text}"</p></view>
        }
    }));
    session.frame(400., 300., 1.);
    session.keyboard(key("tab"));
    session.keyboard(command("a"));
    commit(&mut session, "rejected");
    assert_eq!(value(&session), "accepted");
    assert!(
        session
            .frame(400., 300., 1.)
            .nodes
            .iter()
            .any(|node| node.text.as_deref() == Some("Value: accepted"))
    );
}

#[test]
fn controlled_normalization_and_rejection_sync_without_loop_or_lost_next_edit() {
    use std::cell::Cell;
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let mut session = DesktopSession::new(UiApp::new(move || {
        let text = signal("accepted".to_owned());
        view! {<view>
            <input id="editor" value={text} onInput={move |event| {
                count.set(count.get() + 1);
                if let Event::Input(value) = event {
                    text.set(match value.as_str() { "reject" => "accepted".into(), "normalize" => "NORMALIZE".into(), _ => value });
                }
            }}/><p>"Value: {text}"</p>
        </view>}
    }));
    session.frame(400., 300., 1.);
    session.keyboard(key("tab"));
    for (i, (edit, expected)) in [("reject", "accepted"), ("normalize", "NORMALIZE")]
        .into_iter()
        .enumerate()
    {
        session.keyboard(command("a"));
        commit(&mut session, edit);
        assert_eq!(value(&session), expected);
        for _ in 0..3 {
            assert!(
                session
                    .frame(400., 300., 1.)
                    .nodes
                    .iter()
                    .any(|n| n.text.as_deref() == Some(&format!("Value: {expected}")))
            );
        }
        assert_eq!(calls.get(), i * 2 + 1, "sync must not dispatch input");
        session.keyboard(command("right"));
        commit(&mut session, "!");
        assert_eq!(
            value(&session),
            format!("{expected}!"),
            "editor sync must preserve the accepted value before the next edit"
        );
        session.frame(400., 300., 1.);
        assert_eq!(calls.get(), i * 2 + 2);
    }
}

struct FailingClipboard;
impl TextClipboard for FailingClipboard {
    fn get(&mut self) -> Result<String, String> {
        Err("read failure".into())
    }
    fn set(&mut self, _: &str) -> Result<(), String> {
        Err("write failure".into())
    }
}
#[test]
fn clipboard_failures_reach_app_error_sink_and_empty_paste_keeps_selection() {
    let errors = Rc::new(RefCell::new(vec![]));
    let output = errors.clone();
    let app = UiApp::new_with_error_sink(
        || view! {<input id="editor" value="selected"/>},
        Some(Rc::new(move |e| {
            output
                .borrow_mut()
                .push((e.binding.clone(), e.message.clone()))
        })),
    );
    let mut session = DesktopSession::new(app);
    session.frame(400., 300., 1.);
    session.keyboard(key("tab"));
    session.keyboard(command("a"));
    session.clipboard(FailingClipboard);
    session.keyboard(command("x"));
    session.keyboard(command("v"));
    assert_eq!(value(&session), "selected");
    assert_eq!(
        &*errors.borrow(),
        &[
            ("clipboard".into(), "write failure".into()),
            ("clipboard".into(), "read failure".into())
        ]
    );
    assert_eq!(session.app().take_errors().len(), 2);
    session.clipboard(Clipboard(Rc::new(RefCell::new(String::new()))));
    session.keyboard(command("v"));
    assert_eq!(
        value(&session),
        "selected",
        "empty paste must not delete selection"
    );
    commit(&mut session, "replacement");
    assert_eq!(
        value(&session),
        "replacement",
        "empty paste must preserve selection"
    );
    assert_eq!(errors.borrow().len(), 2);
}
