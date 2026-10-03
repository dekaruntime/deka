//! The two AppKit calls winit does not offer: the window's colour before frame
//! one, and the application menu, built after frame one (winit's default menu
//! costs ~15 ms of launch, spike phase 6).
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{msg_send, sel};
use std::ffi::CString;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

fn class(name: &std::ffi::CStr) -> Option<&'static AnyClass> {
    AnyClass::get(name)
}

/// An autoreleased NSString, or null.
fn ns_string(text: &str) -> *mut AnyObject {
    let (Some(class), Ok(text)) = (class(c"NSString"), CString::new(text)) else {
        return std::ptr::null_mut();
    };
    // SAFETY: stringWithUTF8String: takes a NUL-terminated UTF-8 C string and
    // returns an autoreleased NSString (or nil); `text` outlives the call.
    unsafe { msg_send![class, stringWithUTF8String: text.as_ptr()] }
}

/// Paint the window's background in the application's colour, so the first
/// pixels on screen are already right while frame one is drawn.
pub(crate) fn set_background(window: &Window, rgb: u32) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    let Some(ns_color) = class(c"NSColor") else {
        return;
    };
    let channel = |shift: u32| f64::from((rgb >> shift) & 0xff) / 255.;
    // SAFETY: `ns_view` is the window's live NSView and this runs on the main
    // thread (winit hands out window handles there); every receiver is checked
    // for nil before it is messaged.
    unsafe {
        let view = handle.ns_view.as_ptr().cast::<AnyObject>();
        let ns_window: *mut AnyObject = msg_send![view, window];
        let color: *mut AnyObject = msg_send![
            ns_color,
            colorWithSRGBRed: channel(16),
            green: channel(8),
            blue: channel(0),
            alpha: 1.0f64
        ];
        if let (Some(ns_window), false) = (ns_window.as_ref(), color.is_null()) {
            let _: () = msg_send![ns_window, setBackgroundColor: color];
        }
    }
}

/// NSEventModifierFlags.
const OPTION: usize = 1 << 19;
const COMMAND: usize = 1 << 20;

/// A retained NSMenuItem.
///
/// # Safety
/// Main thread only.
unsafe fn item(
    title: *mut AnyObject,
    action: Option<Sel>,
    key: &str,
    mask: Option<usize>,
) -> *mut AnyObject {
    let Some(class) = class(c"NSMenuItem") else {
        return std::ptr::null_mut();
    };
    // SAFETY: alloc + initWithTitle:action:keyEquivalent: is NSMenuItem's
    // designated initialiser; title and key are NSStrings (nil-checked by the
    // callers through `ns_string`), the action may be NULL.
    unsafe {
        let item: *mut AnyObject = msg_send![class, alloc];
        let item: *mut AnyObject =
            msg_send![item, initWithTitle: title, action: action, keyEquivalent: ns_string(key)];
        if let (Some(item), Some(mask)) = (item.as_ref(), mask) {
            let _: () = msg_send![item, setKeyEquivalentModifierMask: mask];
        }
        item
    }
}

/// The standard application menu: About, Services, Hide, Hide Others, Show
/// All, Quit (Cmd-Q). The same items winit's default menu has.
pub(crate) fn install_menu() {
    let (Some(app_class), Some(menu_class), Some(item_class), Some(info_class)) = (
        class(c"NSApplication"),
        class(c"NSMenu"),
        class(c"NSMenuItem"),
        class(c"NSProcessInfo"),
    ) else {
        return;
    };
    // SAFETY: called from the event loop on the main thread. Objects from
    // `new`/`alloc` are collected in `owned` and released once the menus
    // retain them; other returns are autoreleased or shared singletons. Nil is
    // never messaged (the `as_ref` checks).
    unsafe {
        let app: *mut AnyObject = msg_send![app_class, sharedApplication];
        let info: *mut AnyObject = msg_send![info_class, processInfo];
        let (Some(app), Some(info)) = (app.as_ref(), info.as_ref()) else {
            return;
        };
        let name: *mut AnyObject = msg_send![info, processName];
        let Some(name) = name.as_ref() else { return };
        let titled = |prefix: &str| -> *mut AnyObject {
            match ns_string(prefix).as_ref() {
                Some(prefix) => msg_send![prefix, stringByAppendingString: name],
                None => std::ptr::null_mut(),
            }
        };
        let mut owned: Vec<*mut AnyObject> = vec![];
        let menubar: *mut AnyObject = msg_send![menu_class, new];
        let app_menu: *mut AnyObject = msg_send![menu_class, new];
        let services: *mut AnyObject = msg_send![menu_class, new];
        owned.extend([menubar, app_menu, services]);
        let separator = || -> *mut AnyObject { msg_send![item_class, separatorItem] };
        let mut new_item = |title, action, key, mask| {
            let entry = item(title, action, key, mask);
            owned.push(entry);
            entry
        };
        let app_item = new_item(ns_string(""), None, "", None);
        let services_item = new_item(ns_string("Services"), None, "", None);
        let entries = [
            new_item(
                titled("About "),
                Some(sel!(orderFrontStandardAboutPanel:)),
                "",
                None,
            ),
            separator(),
            services_item,
            new_item(titled("Hide "), Some(sel!(hide:)), "h", None),
            new_item(
                ns_string("Hide Others"),
                Some(sel!(hideOtherApplications:)),
                "h",
                Some(OPTION | COMMAND),
            ),
            new_item(
                ns_string("Show All"),
                Some(sel!(unhideAllApplications:)),
                "",
                None,
            ),
            separator(),
            new_item(titled("Quit "), Some(sel!(terminate:)), "q", None),
        ];
        if let (
            Some(menubar),
            Some(app_menu),
            Some(services),
            Some(app_item),
            Some(services_item),
        ) = (
            menubar.as_ref(),
            app_menu.as_ref(),
            services.as_ref(),
            app_item.as_ref(),
            services_item.as_ref(),
        ) {
            let _: () = msg_send![services_item, setSubmenu: services];
            for entry in entries {
                if let Some(entry) = entry.as_ref() {
                    let _: () = msg_send![app_menu, addItem: entry];
                }
            }
            let _: () = msg_send![app_item, setSubmenu: app_menu];
            let _: () = msg_send![menubar, addItem: app_item];
            let _: () = msg_send![app, setServicesMenu: services];
            let _: () = msg_send![app, setMainMenu: menubar];
        }
        for object in owned {
            if let Some(object) = object.as_ref() {
                let _: () = msg_send![object, release];
            }
        }
    }
}
