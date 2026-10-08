#![cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
use deka_native_ui::window::{KeyInput, MultipleDesktopSession, TextClipboard, WindowToken};
use deka_ui::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, WindowEvent},
};
type Session = MultipleDesktopSession<DesktopApp>;
fn click(session: &mut Session, id: WindowToken, index: usize) {
    let rect = session.frame(id, 1.).unwrap().targets[index].rect;
    let os = session
        .windows()
        .iter()
        .find(|(token, _)| *token == id)
        .unwrap()
        .1;
    session.event(
        os,
        &WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(f64::from(rect.x + 4.), f64::from(rect.y + 4.)),
        },
        1.,
    );
    assert!(session.event(
        os,
        &WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Pressed,
            button: MouseButton::Left
        },
        1.
    ));
    session.event(
        os,
        &WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Released,
            button: MouseButton::Left,
        },
        1.,
    );
}
fn text(session: &mut Session, id: WindowToken) -> Vec<String> {
    session
        .frame(id, 1.)
        .unwrap()
        .nodes
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}
#[test]
fn os_handlers_open_close_and_share_state_without_closing_other_windows() {
    let shared = Rc::new(Cell::new(None));
    let output = shared.clone();
    let local = Rc::new(Cell::new(None));
    let local_output = local.clone();
    let app = DesktopApp::new(move |windows| {
        let mut count = signal(0);
        output.set(Some(count));
        let opener = windows.clone();
        windows.open(WindowOptions::new("A",320.,260.),move|window| {
            local_output.set(Some(signal(7)));
            view! {<view><p>"Count: {count}"</p>
                <button onClick={move |_|count+=1}>"Add"</button>
                <button onClick={move |_|{opener.open(WindowOptions::new("C",320.,260.),move|child|view!{<view><p>"Count: {count}"</p><button onClick={move |_|{child.close();}}>"Close"</button></view>}).unwrap();}}>"Open"</button>
                <button onClick={move |_|{window.close();}}>"Close"</button>
            </view>}
        }).unwrap();
        windows.open(WindowOptions::new("B",360.,280.),move|window|view!{
            <view><p>"Count: {count}"</p><button onClick={move |_|count+=1}>"Add"</button><button onClick={move |_|{window.close();}}>"Close"</button></view>
        }).unwrap();
    });
    let mut session = Session::new(app);
    let windows = session.windows();
    assert_eq!(windows.len(), 2);
    let (a, os_a) = windows[0];
    let (b, os_b) = windows[1];
    assert!(text(&mut session, a).contains(&"Count: 0".into()));
    assert!(text(&mut session, b).contains(&"Count: 0".into()));
    click(&mut session, a, 0);
    assert!(
        session.turn(b),
        "shared updates wake the other window's render turn"
    );
    assert!(text(&mut session, b).contains(&"Count: 1".into()));
    click(&mut session, a, 1);
    assert_eq!(session.windows().len(), 3);
    let c = session
        .windows()
        .iter()
        .find(|(id, _)| *id != a && *id != b)
        .unwrap()
        .0;
    assert!(text(&mut session, c).contains(&"Count: 1".into()));
    click(&mut session, c, 0);
    assert_eq!(session.windows().len(), 2);
    assert!(session.event(os_a, &WindowEvent::CloseRequested, 1.));
    assert_eq!(
        local.get().unwrap().try_get(),
        Err(ReactiveError::DisposedSignal)
    );
    assert!(!session.event(
        os_a,
        &WindowEvent::Ime(winit::event::Ime::Commit("stale".into())),
        1.
    ));
    click(&mut session, b, 0);
    assert_eq!(shared.get().unwrap().get(), Ok(2));
    assert!(text(&mut session, b).contains(&"Count: 2".into()));
    assert!(session.event(os_b, &WindowEvent::CloseRequested, 1.));
    assert!(session.windows().is_empty());
}
#[test]
fn shared_reactions_keep_their_originating_tree_and_event_allocations_drop_on_close() {
    let observations = Rc::new(RefCell::new(vec![]));
    let output = observations.clone();
    let local = Rc::new(Cell::new(None));
    let local_out = local.clone();
    let app = DesktopApp::new(move |windows| {
        let mut trigger = signal(0);
        for name in ["A", "B"] {
            let output = output.clone();
            let local = local_out.clone();
            windows.open(WindowOptions::new(name,320.,200.),move|_| {
                effect(move||{if trigger.get().unwrap()>0 {output.borrow_mut().push(tree().unwrap().get_element_by_id("self").unwrap().text_content());}});
                view!{<view><p id="self">{name}</p><button onClick={move |_|{local.set(Some(signal(42)));trigger+=1;tree().unwrap().get_element_by_id("self").unwrap().set_text_content(format!("{name} clicked")).unwrap();}}>"Add"</button></view>}
            }).unwrap();
        }
    });
    let mut session = Session::new(app);
    let windows = session.windows();
    let (a, os_a) = windows[0];
    let (b, _) = windows[1];
    session.frame(a, 1.);
    session.frame(b, 1.);
    click(&mut session, a, 0);
    assert_eq!(&*observations.borrow(), &["A clicked", "B"]);
    assert!(text(&mut session, b).contains(&"B".into()));
    session.event(os_a, &WindowEvent::CloseRequested, 1.);
    assert_eq!(
        local.get().unwrap().try_get(),
        Err(ReactiveError::DisposedSignal)
    );
}
#[test]
fn node_refs_remain_foreign_between_windows_in_the_same_app_scope() {
    let reference = node_ref();
    let output = reference.clone();
    let app = DesktopApp::new(move |windows| {
        for name in ["A", "B"] {
            let reference = reference.clone();
            windows
                .open(
                    WindowOptions::new(name, 320., 200.),
                    move |_| view! {<input node_ref={reference}/>},
                )
                .unwrap();
        }
    });
    let mut session = Session::new(app);
    let windows = session.windows();
    let (a, os_a) = windows[0];
    let (b, _) = windows[1];
    assert!(output.get().is_some());
    assert!(
        session
            .app(b)
            .unwrap()
            .take_errors()
            .iter()
            .any(|error| error.contains("another tree session"))
    );
    session.event(os_a, &WindowEvent::CloseRequested, 1.);
    assert!(output.get().is_none());
    assert!(session.app(a).is_none());
}
struct Clipboard(Rc<RefCell<String>>);
impl TextClipboard for Clipboard {
    fn get(&mut self) -> Result<String, String> {
        Ok(self.0.borrow().clone())
    }
    fn set(&mut self, text: &str) -> Result<(), String> {
        *self.0.borrow_mut() = text.into();
        Ok(())
    }
}
fn command(name: &str) -> KeyInput {
    KeyInput {
        name: name.into(),
        command: true,
        down: true,
        ..Default::default()
    }
}
#[test]
fn clipboard_owner_outlives_the_window_that_copied_text() {
    let app = DesktopApp::new(|windows| {
        for value in ["A", "B"] {
            windows
                .open(
                    WindowOptions::new(value, 320., 200.),
                    move |_| view! {<input id="editor" value={value}/>},
                )
                .unwrap();
        }
    });
    let mut session = Session::new(app);
    let clipboard = Rc::new(RefCell::new(String::new()));
    session.clipboard(Clipboard(clipboard.clone()));
    let windows = session.windows();
    let (a, os_a) = windows[0];
    let (b, os_b) = windows[1];
    session.frame(a, 1.);
    session.keyboard(
        os_a,
        KeyInput {
            name: "tab".into(),
            down: true,
            ..Default::default()
        },
    );
    session.keyboard(os_a, command("a"));
    session.keyboard(os_a, command("c"));
    assert_eq!(&*clipboard.borrow(), "A");
    session.event(os_a, &WindowEvent::CloseRequested, 1.);
    session.frame(b, 1.);
    session.keyboard(
        os_b,
        KeyInput {
            name: "tab".into(),
            down: true,
            ..Default::default()
        },
    );
    session.keyboard(os_b, command("a"));
    session.keyboard(os_b, command("v"));
    assert_eq!(
        session
            .app(b)
            .unwrap()
            .tree()
            .get_element_by_id("editor")
            .unwrap()
            .get_attribute("value")
            .as_deref(),
        Some("A")
    );
}

#[test]
fn a_window_closed_while_pending_never_runs_its_factory() {
    let factories = Rc::new(Cell::new(0));
    let output = factories.clone();
    let app = DesktopApp::new(move |windows| {
        let closed = windows
            .open(WindowOptions::new("Never mounted", 200., 100.), move |_| {
                output.set(output.get() + 1);
                view! {<p>"Must not mount"</p>}
            })
            .unwrap();
        assert!(closed.close());
        windows
            .open(
                WindowOptions::new("Survivor", 200., 100.),
                |_| view! {<p>"Alive"</p>},
            )
            .unwrap();
    });
    let mut session = Session::new(app);
    assert_eq!(factories.get(), 0);
    assert_eq!(session.windows().len(), 1);
    let id = session.windows()[0].0;
    assert!(text(&mut session, id).contains(&"Alive".into()));
}

#[test]
fn closing_a_window_drops_its_actual_shared_signal_subscriber() {
    let trigger = Rc::new(Cell::new(None));
    let output = trigger.clone();
    let counts = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
    let callbacks = counts.clone();
    let app = DesktopApp::new(move |windows| {
        let shared = signal(0);
        output.set(Some(shared));
        for (i, count) in callbacks.into_iter().enumerate() {
            windows
                .open(
                    WindowOptions::new(format!("Window {i}"), 200., 100.),
                    move |_| {
                        effect(move || {
                            let _ = shared.get().unwrap();
                            count.set(count.get() + 1);
                        });
                        view! {<p>"Subscriber"</p>}
                    },
                )
                .unwrap();
        }
    });
    let mut session = Session::new(app);
    let windows = session.windows();
    assert_eq!([counts[0].get(), counts[1].get()], [1, 1]);
    session.event(windows[0].1, &WindowEvent::CloseRequested, 1.);
    trigger.get().unwrap().set(1);
    assert_eq!(
        [counts[0].get(), counts[1].get()],
        [1, 2],
        "closed mount's subscriber must be removed, surviving subscriber must still run"
    );
    session.event(windows[1].1, &WindowEvent::CloseRequested, 1.);
    trigger.get().unwrap().set(2);
    assert_eq!([counts[0].get(), counts[1].get()], [1, 2]);
}

#[test]
fn failed_window_uses_the_same_app_sink_as_window_runtime_errors() {
    use deka_native_ui::window::WindowController;
    let errors = Rc::new(RefCell::new(vec![]));
    let output = errors.clone();
    let mut failed = None;
    let mut app = DesktopApp::new_with_error_sink(
        |windows| {
            failed = Some(
                windows
                    .open(WindowOptions::new("A", 200., 100.), |_| view! {<p>"A"</p>})
                    .unwrap()
                    .id(),
            );
            windows
                .open(WindowOptions::new("B", 200., 100.), |_| {
                    View::element("view").attr("class", "invalid-utility")
                })
                .unwrap();
        },
        Some(Rc::new(move |error| {
            output
                .borrow_mut()
                .push((error.binding.clone(), error.message.clone()))
        })),
    );
    app.failed(failed.unwrap(), "cannot create native window".into());
    let requests = app.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        errors.borrow().len(),
        2,
        "open failure and runtime mount failure share one sink"
    );
    assert_eq!(errors.borrow()[0].0, "window");
    assert!(errors.borrow()[0].1.contains("cannot create native window"));
    assert_eq!(app.take_errors().len(), 1);
}
