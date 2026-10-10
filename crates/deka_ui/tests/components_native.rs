#![cfg(all(feature = "desktop", feature = "tour", not(target_arch = "wasm32")))]
use deka_native_ui::{
    Application,
    window::{DesktopSession, KeyInput, TextClipboard, accesskit_events::Role},
};
use deka_ui::{components::*, prelude::*};
use std::{cell::RefCell, rc::Rc};
use winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, Ime, MouseButton, WindowEvent},
};

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
fn click(session: &mut DesktopSession<UiApp>, label: &str) {
    let id = session
        .app()
        .semantics()
        .into_iter()
        .find(|n| n.name == label && n.tab_index.is_some())
        .unwrap()
        .id;
    let rect = session
        .frame(560., 480., 1.)
        .nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap()
        .rect;
    let device = DeviceId::dummy();
    session.event(
        &WindowEvent::CursorMoved {
            device_id: device,
            position: PhysicalPosition::new((rect.x + 8.).into(), (rect.y + 8.).into()),
        },
        1.,
    );
    assert!(session.event(
        &WindowEvent::MouseInput {
            device_id: device,
            state: ElementState::Pressed,
            button: MouseButton::Left
        },
        1.
    ));
    session.event(
        &WindowEvent::MouseInput {
            device_id: device,
            state: ElementState::Released,
            button: MouseButton::Left,
        },
        1.,
    );
}
fn named_id(session: &DesktopSession<UiApp>, name: &str) -> String {
    session
        .app()
        .semantics()
        .into_iter()
        .find(|n| n.name == name && n.tab_index.is_some())
        .unwrap()
        .id
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

#[test]
fn native_button_list_and_tabs_use_real_pointer_keyboard_and_focus_ingress() {
    let mut session = DesktopSession::new(deka_ui::tour::app("component-button-light").unwrap());
    session.frame(560., 480., 1.);
    click(&mut session, "Add one");
    assert!(session.keyboard(key("space")));
    assert!(session.keyboard(key("enter")));
    assert!(
        session
            .frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Count: 3"))
    );
    session.keyboard(key("tab"));
    assert_eq!(session.focus(), None, "disabled button must be skipped");

    let mut list = DesktopSession::new(deka_ui::tour::app("component-list-light").unwrap());
    list.frame(560., 480., 1.);
    click(&mut list, "Zega");
    let selected_id = named_id(&list, "Zega");
    assert_eq!(list.focus(), Some(selected_id.as_str()));
    click(&mut list, "Reverse order");
    list.frame(560., 480., 1.);
    assert_eq!(named_id(&list, "Zega"), selected_id);
    list.keyboard(KeyInput {
        shift: true,
        ..key("tab")
    });
    assert_eq!(list.focus(), Some(named_id(&list, "Deka").as_str()));
    assert!(list.keyboard(key("enter")));
    assert!(
        list.frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Selected: deka"))
    );

    let mut tabs = DesktopSession::new(deka_ui::tour::app("component-tabs-dark").unwrap());
    tabs.frame(560., 480., 1.);
    tabs.keyboard(key("tab"));
    assert_eq!(tabs.focus(), Some(named_id(&tabs, "Overview").as_str()));
    assert!(tabs.keyboard(key("right")));
    assert_eq!(tabs.focus(), Some(named_id(&tabs, "Activity").as_str()));
    assert!(
        tabs.frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Your latest project activity."))
    );
    assert!(tabs.keyboard(key("end")));
    assert_eq!(tabs.focus(), Some(named_id(&tabs, "Settings").as_str()));
    assert!(tabs.keyboard(key("right")));
    assert_eq!(tabs.focus(), Some(named_id(&tabs, "Overview").as_str()));
    click(&mut tabs, "Activity");
    assert!(tabs.keyboard(key("home")));
    assert_eq!(tabs.focus(), Some(named_id(&tabs, "Overview").as_str()));
    let projected = tabs.accessibility(1.);
    assert_eq!(
        projected
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == Role::TabList)
            .count(),
        1
    );
    let overview = projected
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::Tab && n.label() == Some("Overview"))
        .unwrap();
    assert_eq!(overview.1.is_selected(), Some(true));
    assert_eq!(overview.1.controls().len(), 1);
}

#[test]
fn native_input_selection_copy_cut_paste_single_line_and_signal_updates() {
    let mut value = None;
    let app = UiApp::new(|| {
        let text = signal("hello 日本".to_owned());
        value = Some(text);
        view! {<view class="p-4 gap-3">
            <Input id="editor" label="Name" value={text}/>
            <Button label="Clear" on_press={Rc::new(move ||text.set(String::new()))}/>
            <p>"Hello {text}"</p>
        </view>}
    });
    let mut session = DesktopSession::new(app);
    let clipboard = Rc::new(RefCell::new(String::new()));
    session.clipboard(Clipboard(clipboard.clone()));
    session.frame(560., 480., 1.);
    click(&mut session, "Name");
    assert_eq!(session.focus(), Some(named_id(&session, "Name").as_str()));
    assert!(session.keyboard(command("a")));
    assert!(session.keyboard(command("c")));
    assert_eq!(&*clipboard.borrow(), "hello 日本");
    assert!(session.keyboard(command("x")));
    assert_eq!(value.unwrap().get().unwrap(), "");
    assert!(session.keyboard(command("v")));
    assert_eq!(value.unwrap().get().unwrap(), "hello 日本");
    session.frame(560., 480., 1.);
    session.keyboard(command("a"));
    assert!(session.event(&WindowEvent::Ime(Ime::Commit("Sami\nFouad".into())), 1.));
    assert_eq!(value.unwrap().get().unwrap(), "SamiFouad");
    assert!(
        session
            .frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Hello SamiFouad"))
    );
    session.keyboard(key("tab"));
    assert_eq!(session.focus(), Some(named_id(&session, "Clear").as_str()));
    session.keyboard(key("enter"));
    assert_eq!(value.unwrap().get().unwrap(), "");
    session.keyboard(KeyInput {
        shift: true,
        ..key("tab")
    });
    assert_eq!(session.focus(), Some(named_id(&session, "Name").as_str()));
}

#[test]
fn offscreen_light_dark_screenshots_for_every_component() {
    use deka_native_ui::snapshot;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tmp/components-shots");
    std::fs::create_dir_all(&out).unwrap();
    for showcase in deka_ui::tour::SHOWCASES {
        let mut frames = vec![];
        for (name, theme) in [("light", Theme::Light), ("dark", Theme::Dark)] {
            let app = (showcase.app)(theme);
            assert!(app.take_errors().is_empty());
            let mut session = DesktopSession::new(app);
            session.frame(560., 480., 1.);
            if showcase.id == "dialog" {
                click(&mut session, "Open dialog");
            }
            if showcase.id == "menu" {
                click(&mut session, "Actions");
            }
            let scene = session.frame(560., 480., 1.).clone();
            let surface = match showcase.id {
                "badge" => Some((deka_native_ui::SemanticRole::Status, 40.)),
                "toast" => Some((deka_native_ui::SemanticRole::Status, 140.)),
                "dialog" => Some((deka_native_ui::SemanticRole::Dialog, 300.)),
                "menu" => Some((deka_native_ui::SemanticRole::Menu, 180.)),
                _ => None,
            };
            if let Some((role, max_height)) = surface {
                let id = session
                    .app()
                    .semantics()
                    .into_iter()
                    .find(|n| n.role == role)
                    .unwrap()
                    .id;
                let rect = scene.nodes.iter().find(|n| n.id == id).unwrap().rect;
                assert!(
                    rect.height > 0. && rect.height <= max_height,
                    "{} surface must keep intrinsic height: {}",
                    showcase.id,
                    rect.height
                );
                if showcase.id == "badge" {
                    assert!(rect.width < 280., "badge keeps content width");
                }
            }
            if showcase.id == "input" {
                let foreground =
                    u32::from_str_radix(theme.tokens().foreground.trim_start_matches('#'), 16)
                        .unwrap();
                assert_eq!(session.app().text_controls()[0].value, "Sami");
                assert_eq!(session.app().text_controls()[0].color, foreground);
                let editor_id = &session.app().text_controls()[0].id;
                let editor = scene
                    .nodes
                    .iter()
                    .find(|node| &node.id == editor_id)
                    .unwrap();
                assert!(
                    scene.paint.iter().any(|paint| paint.image.is_some()
                        && paint.color == foreground
                        && editor.rect.contains(paint.rect.x, paint.rect.y)),
                    "the native editor must paint its value in the theme foreground"
                );
            }
            let shot = snapshot(&scene, 1.).expect("production GPU must render offscreen");
            assert_eq!((shot.width, shot.height), (560, 480));
            let expected =
                u32::from_str_radix(theme.tokens().background.trim_start_matches('#'), 16).unwrap();
            assert_eq!(
                shot.pixel(2, 2),
                [
                    (expected >> 16) as u8,
                    (expected >> 8) as u8,
                    expected as u8,
                    255
                ]
            );
            assert!(
                shot.rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| *pixel != shot.pixel(2, 2)),
                "component paints visible pixels"
            );
            std::fs::write(out.join(format!("{}-{name}.rgba", showcase.id)), &shot.rgba).unwrap();
            frames.push(shot.rgba);
        }
        assert_ne!(frames[0], frames[1], "light and dark must differ");
    }
}

#[test]
fn showcase_tabs_keep_intrinsic_height_and_room_for_content() {
    let app = deka_ui::tour::app("component-tabs-light").unwrap();
    let scene =
        deka_native_ui::scene::Renderer::new().render_at(&app.tree(), 560., 480., 1., 0., true);
    for target in &scene.targets {
        assert!(
            target.rect.height <= 48.,
            "tab must keep control height, got {}",
            target.rect.height
        );
    }
    let panel_text = scene
        .nodes
        .iter()
        .find(|node| node.text.as_deref() == Some("Everything you need in one place."))
        .unwrap()
        .rect;
    let following_text = scene
        .nodes
        .iter()
        .find(|node| node.text.as_deref() == Some("Selected: overview"))
        .unwrap()
        .rect;
    assert!(
        following_text.y - (panel_text.y + panel_text.height) <= 40.,
        "inactive panels must not push following content down"
    );
}

#[test]
fn native_badge_and_toast_pointer_keyboard_signal_focus_and_status_roles() {
    let mut badge = DesktopSession::new(deka_ui::tour::app("component-badge-dark").unwrap());
    badge.frame(560., 480., 1.);
    badge.keyboard(key("tab"));
    assert_eq!(
        badge.focus(),
        Some(named_id(&badge, "Update status").as_str())
    );
    badge.keyboard(key("enter"));
    assert!(
        badge
            .frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Published"))
    );
    assert!(
        badge
            .accessibility(1.)
            .nodes
            .iter()
            .any(|(_, n)| n.role() == Role::Status)
    );

    let mut toast = DesktopSession::new(deka_ui::tour::app("component-toast-light").unwrap());
    toast.frame(560., 480., 1.);
    toast.keyboard(key("tab"));
    toast.keyboard(key("tab"));
    assert_eq!(
        toast.focus(),
        Some(named_id(&toast, "Dismiss notification").as_str())
    );
    toast.keyboard(key("space"));
    assert!(
        !toast
            .frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Your changes are saved."))
    );
    click(&mut toast, "Show notification");
    toast.frame(560., 480., 1.);
    click(&mut toast, "Dismiss notification");
    assert!(
        !toast
            .frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Your changes are saved."))
    );
}

#[test]
fn native_dialog_traps_focus_blocks_background_and_escape_from_editor_restores_opener() {
    let mut session = DesktopSession::new(deka_ui::tour::app("component-dialog-dark").unwrap());
    session.frame(560., 480., 1.);
    let opener = named_id(&session, "Open dialog");
    let accessible_opener = session
        .accessibility(1.)
        .nodes
        .into_iter()
        .find(|(_, n)| n.label() == Some("Open dialog"))
        .unwrap()
        .0;
    click(&mut session, "Open dialog");
    session.frame(560., 480., 1.);
    let first = named_id(&session, "Profile name");
    let close = named_id(&session, "Close dialog");
    assert_eq!(session.focus(), Some(first.as_str()));
    use deka_native_ui::window::accesskit_events::{
        Action, ActionRequest, TreeId, WindowEvent as AccessEvent,
    };
    for action in [Action::Focus, Action::Click] {
        assert!(
            !session.accessibility_event(&AccessEvent::ActionRequested(ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node: accessible_opener,
                data: None
            }))
        );
    }

    assert!(
        session
            .accessibility(1.)
            .nodes
            .iter()
            .any(|(_, n)| n.role() == Role::Dialog && n.is_modal())
    );
    session.keyboard(KeyInput {
        shift: true,
        ..key("tab")
    });
    assert_eq!(session.focus(), Some(close.as_str()));
    session.keyboard(key("tab"));
    assert_eq!(session.focus(), Some(first.as_str()));
    session.keyboard(command("a"));
    session.event(&WindowEvent::Ime(Ime::Commit("Ava".into())), 1.);
    assert!(
        session
            .frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Hello Ava"))
    );
    assert!(
        session
            .app()
            .semantics()
            .iter()
            .find(|n| n.id == opener)
            .unwrap()
            .hidden
    );
    session.keyboard(key("escape"));
    session.frame(560., 480., 1.);
    assert_eq!(session.focus(), Some(opener.as_str()));
    click(&mut session, "Open dialog");
    session.frame(560., 480., 1.);
    click(&mut session, "Close dialog");
    session.frame(560., 480., 1.);
    assert_eq!(session.focus(), Some(opener.as_str()));
}

#[test]
fn native_menu_arrow_wrap_disabled_selection_and_escape_restore() {
    let mut session = DesktopSession::new(deka_ui::tour::app("component-menu-light").unwrap());
    session.frame(560., 480., 1.);
    session.keyboard(key("tab"));
    let trigger = named_id(&session, "Actions");
    session.keyboard(key("down"));
    session.frame(560., 480., 1.);
    assert_eq!(session.focus(), Some(named_id(&session, "Edit").as_str()));
    session.keyboard(key("up"));
    assert_eq!(
        session.focus(),
        Some(named_id(&session, "Duplicate").as_str())
    );
    assert!(
        session
            .accessibility(1.)
            .nodes
            .iter()
            .any(|(_, n)| n.role() == Role::MenuItem)
    );
    session.keyboard(key("enter"));
    assert_eq!(session.focus(), Some(trigger.as_str()));
    assert!(
        session
            .frame(560., 480., 1.)
            .nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Selected: Duplicate"))
    );
    click(&mut session, "Actions");
    session.frame(560., 480., 1.);
    session.keyboard(key("escape"));
    session.frame(560., 480., 1.);
    assert_eq!(session.focus(), Some(trigger.as_str()));
    assert!(
        !session
            .app()
            .semantics()
            .iter()
            .any(|n| n.role == deka_native_ui::SemanticRole::Menu)
    );
}
