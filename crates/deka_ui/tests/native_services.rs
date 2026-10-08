#![cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
use deka_native_ui::window::{
    DialogKind, FileDialogs, MultipleDesktopSession, NativeWindow, WindowToken,
};
use deka_ui::prelude::*;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
use winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, WindowEvent},
};
type Session = MultipleDesktopSession<DesktopApp>;
fn mouse(session: &mut Session, window: WindowToken, point: (f32, f32), button: MouseButton) {
    let os = session
        .windows()
        .iter()
        .find(|(id, _)| *id == window)
        .unwrap()
        .1;
    session.event(
        os,
        &WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(f64::from(point.0), f64::from(point.1)),
        },
        1.,
    );
    session.event(
        os,
        &WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Pressed,
            button,
        },
        1.,
    );
    session.event(
        os,
        &WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Released,
            button,
        },
        1.,
    );
}
fn click(session: &mut Session, window: WindowToken, index: usize) {
    let rect = session.frame(window, 1.).unwrap().targets[index].rect;
    mouse(
        session,
        window,
        (rect.x + 4., rect.y + 4.),
        MouseButton::Left,
    );
}
fn texts(session: &mut Session, window: WindowToken) -> Vec<String> {
    session
        .frame(window, 1.)
        .unwrap()
        .nodes
        .iter()
        .filter_map(|node| node.text.clone())
        .collect()
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn menu_os_payloads_change_all_windows_and_right_click_keeps_origin_and_rejects_stale_actions() {
    use deka_native_ui::window::{MenuEvent, MenuId};
    let ids = Rc::new(RefCell::new(Vec::<MenuId>::new()));
    let output = ids.clone();
    let shared = Rc::new(Cell::new(None));
    let shared_out = shared.clone();
    let app = DesktopApp::new(move |windows| {
        let mut count = signal(0);
        shared_out.set(Some(count));
        let add = MenuItem::new("Add", move || count += 1)
            .accelerator("CmdOrCtrl+I")
            .unwrap();
        let disabled = MenuItem::new("Disabled", move || count += 100).enabled(false);
        output
            .borrow_mut()
            .extend([add.id().clone(), disabled.id().clone()]);
        windows
            .app_menu(
                Menu::new()
                    .standard_app("deka")
                    .submenu("Actions", Menu::new().item(add).separator().item(disabled)),
            )
            .unwrap();
        for name in ["First", "Second"] {
            let ids = output.clone();
            windows.open(WindowOptions::new(name,360.,240.),move|window|view! {
    <view className="p-4 gap-4" onContextMenu={move|event|if let Event::ContextMenu{x,y}=event {
     let action=MenuItem::new("Context add",move||{count+=1;tree().unwrap().get_element_by_id("origin").unwrap().set_text_content(format!("{name} context")).unwrap();});
     ids.borrow_mut().push(action.id().clone());window.context_menu(Menu::new().item(action),x,y).unwrap();
    }}><p id="origin">{name}</p><p>"Count: {count}"</p></view>
   }).unwrap();
        }
    });
    let mut session = Session::new(app);
    let windows = session.windows();
    let (first, os_first) = windows[0];
    let (second, _) = windows[1];
    session.frame(first, 1.);
    session.frame(second, 1.);
    assert!(session.menu_event(MenuEvent {
        id: ids.borrow()[0].clone()
    }));
    assert!(texts(&mut session, first).contains(&"Count: 1".into()));
    assert!(texts(&mut session, second).contains(&"Count: 1".into()));
    assert!(!session.menu_event(MenuEvent {
        id: ids.borrow()[1].clone()
    }));
    assert_eq!(shared.get().unwrap().get(), Ok(1));
    let rect = session
        .frame(first, 1.)
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.text.as_deref() == Some("First"))
        .unwrap()
        .rect;
    mouse(
        &mut session,
        first,
        (rect.x + 4., rect.y + 4.),
        MouseButton::Right,
    );
    assert_eq!(
        ids.borrow().len(),
        3,
        "right mouse event bubbles from text to its view handler"
    );
    let context_id = ids.borrow()[2].clone();
    assert!(session.menu_event(MenuEvent {
        id: context_id.clone()
    }));
    assert!(texts(&mut session, first).contains(&"First context".into()));
    assert!(texts(&mut session, second).contains(&"Second".into()));
    assert!(texts(&mut session, second).contains(&"Count: 2".into()));
    let rect = session
        .frame(first, 1.)
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.text.as_deref() == Some("First context"))
        .unwrap()
        .rect;
    mouse(
        &mut session,
        first,
        (rect.x + 4., rect.y + 4.),
        MouseButton::Right,
    );
    let stale = ids.borrow().last().unwrap().clone();
    session.event(os_first, &WindowEvent::CloseRequested, 1.);
    assert!(!session.menu_event(MenuEvent { id: stale }));
    assert_eq!(shared.get().unwrap().get(), Ok(2));
    assert!(session.service_errors().is_empty());
}
struct Dialogs(Rc<Cell<usize>>);
impl FileDialogs for Dialogs {
    fn choose(
        &mut self,
        kind: DialogKind,
        options: &FileDialogOptions,
        parent: Option<&NativeWindow>,
    ) -> DialogResult {
        assert!(
            parent.is_none(),
            "headless provider never opens an OS window"
        );
        assert_eq!(
            options.filters,
            vec![("Text".into(), vec!["txt".into(), "md".into()])]
        );
        let call = self.0.get();
        self.0.set(call + 1);
        match call {
            0 => {
                assert_eq!(kind, DialogKind::Open);
                Ok(Some(PathBuf::from("/documents/selected.md")))
            }
            1 => {
                assert_eq!(kind, DialogKind::Save);
                assert_eq!(options.file_name.as_deref(), Some("draft.txt"));
                Ok(None)
            }
            _ => Err("backend unavailable".into()),
        }
    }
}
#[test]
fn os_clicks_queue_filtered_dialogs_results_repaint_and_cancel_preserves_selection() {
    let calls = Rc::new(Cell::new(0));
    let local = Rc::new(Cell::new(None));
    let local_out = local.clone();
    let app = DesktopApp::new(move |windows| {
        let selected = signal("No file".to_owned());
        let status = signal("Ready".to_owned());
        windows.open(WindowOptions::new("Picker",360.,320.),move|window| {
   let open=window.clone();let save=window.clone();let local=local_out.clone();
   view! {<view className="p-4 gap-4"><p id="origin">"Origin"</p><p>{selected}</p><p>{status}</p>
    <button onClick={move |_|{let local=local.clone();open.open_file(FileDialogOptions::new().filter("Text",&["txt","md"]),move|result|{
      local.set(Some(signal(42)));tree().unwrap().get_element_by_id("origin").unwrap().set_text_content("Completed here").unwrap();
      match result {Ok(Some(path))=>{selected.set(path.display().to_string());status.set("Opened".into());},Ok(None)=>{status.set("Cancelled".into());},Err(error)=>{status.set(error);}}
    }).unwrap();}}>"Open"</button>
    <button onClick={move |_|{save.save_file(FileDialogOptions::new().file_name("draft.txt").filter("Text",&["txt","md"]),move|result|{
      if let Ok(Some(path))=result {selected.set(path.display().to_string());}else{status.set("Cancelled".into());}
    }).unwrap();}}>"Save"</button>
   </view>}
  }).unwrap();
        windows
            .open(
                WindowOptions::new("Shared result", 360., 180.),
                move |_| view! {<view><p>{selected}</p><p>{status}</p></view>},
            )
            .unwrap();
    });
    let mut session = Session::new(app);
    session.dialogs(Dialogs(calls.clone()));
    let windows = session.windows();
    let (picker, os_picker) = windows[0];
    let (other, _) = windows[1];
    click(&mut session, picker, 0);
    assert_eq!(calls.get(), 1);
    assert!(texts(&mut session, picker).contains(&"Completed here".into()));
    assert!(texts(&mut session, other).contains(&"/documents/selected.md".into()));
    click(&mut session, picker, 1);
    assert_eq!(calls.get(), 2);
    assert!(texts(&mut session, other).contains(&"Cancelled".into()));
    assert!(texts(&mut session, other).contains(&"/documents/selected.md".into()));
    click(&mut session, picker, 0);
    assert_eq!(calls.get(), 3);
    assert!(texts(&mut session, other).contains(&"backend unavailable".into()));
    session.event(os_picker, &WindowEvent::CloseRequested, 1.);
    assert_eq!(
        local.get().unwrap().try_get(),
        Err(ReactiveError::DisposedSignal)
    );
    assert_eq!(
        session.service_errors(),
        &["backend unavailable".to_owned()]
    );
}

#[test]
fn invalid_native_filters_report_error_through_os_click_without_opening_a_panel() {
    let app = DesktopApp::new(|windows| {
        let status = signal("Ready".to_owned());
        windows.open(WindowOptions::new("Invalid filter", 320., 180.), move |window| view! {
            <view><p>{status}</p><button onClick={move |_| {
                window.open_file(FileDialogOptions::new().filter("Empty", &[]), move |result| {
                    status.set(result.unwrap_err());
                }).unwrap();
            }}>"Open"</button></view>
        }).unwrap();
    });
    let mut session = Session::new(app);
    session.dialogs(deka_native_ui::window::SystemFileDialogs);
    let id = session.windows()[0].0;
    click(&mut session, id, 0);
    assert!(texts(&mut session, id).contains(&"file filters require nonempty extensions".into()));
}

#[test]
fn native_option_failures_reach_the_result_callback_and_app_error_sink_once() {
    let errors = Rc::new(RefCell::new(vec![]));
    let output = errors.clone();
    let app = DesktopApp::new_with_error_sink(
        |windows| {
            let status = signal("Ready".to_owned());
            windows.open(WindowOptions::new("Guard",320.,180.),move|window| view!{
            <view><p>{status}</p><button onClick={move |_|{
                window.open_file(FileDialogOptions::new().title("bad\0title"),move|result|status.set(result.unwrap_err())).unwrap();
            }}>"Open"</button></view>
        }).unwrap();
        },
        Some(Rc::new(move |error| {
            output
                .borrow_mut()
                .push((error.binding.clone(), error.message.clone()))
        })),
    );
    let mut session = Session::new(app);
    session.dialogs(deka_native_ui::window::SystemFileDialogs);
    let id = session.windows()[0].0;
    click(&mut session, id, 0);
    assert!(
        texts(&mut session, id)
            .contains(&"file dialog options cannot contain NUL characters".into())
    );
    assert_eq!(errors.borrow().len(), 1);
    assert_eq!(errors.borrow()[0].0, "native service");
    assert_eq!(session.service_errors().len(), 1);
}

#[test]
fn right_click_payload_respects_hidden_and_disabled_ancestors_and_false_values() {
    use deka_native_ui::Application;
    for attribute in ["aria-hidden", "disabled"] {
        let count = Rc::new(Cell::new(0));
        let output = count.clone();
        let app = DesktopApp::new(move |windows| {
            windows.open(WindowOptions::new("Context",320.,180.),move|_|view!{
                <view id="parent" onContextMenu={move |_|output.set(output.get()+1)}><p>"Target"</p></view>
            }).unwrap();
        });
        let mut session = Session::new(app);
        let id = session.windows()[0].0;
        let rect = session
            .frame(id, 1.)
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.text.as_deref() == Some("Target"))
            .unwrap()
            .rect;
        let point = (rect.x + 2., rect.y + 2.);
        mouse(&mut session, id, point, MouseButton::Right);
        assert_eq!(count.get(), 1);
        session
            .app(id)
            .unwrap()
            .tree()
            .get_element_by_id("parent")
            .unwrap()
            .set_attribute(attribute, "true")
            .unwrap();
        mouse(&mut session, id, point, MouseButton::Right);
        assert_eq!(
            count.get(),
            1,
            "queued right clicks must recheck ancestor state before repaint"
        );
        session
            .app(id)
            .unwrap()
            .tree()
            .get_element_by_id("parent")
            .unwrap()
            .set_attribute(attribute, "false")
            .unwrap();
        mouse(&mut session, id, point, MouseButton::Right);
        assert_eq!(count.get(), 2);
        if attribute == "disabled" {
            session
                .app(id)
                .unwrap()
                .tree()
                .get_element_by_id("parent")
                .unwrap()
                .set_attribute(attribute, "0")
                .unwrap();
            mouse(&mut session, id, point, MouseButton::Right);
            assert_eq!(count.get(), 3);
            assert!(
                !session
                    .app(id)
                    .unwrap()
                    .semantics()
                    .iter()
                    .any(|n| n.disabled)
            );
        }
    }
}
