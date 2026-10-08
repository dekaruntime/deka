//! Window input in deka's own terms, independent of the windowing crate.
//! Key names are the ones the GPUI adapter delivered (`"tab"`, `"up"`, `"m"`),
//! which the applications and the portfolio world already match on.
use winit::keyboard::{Key, NamedKey};

/// One input event, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input {
    EditKey(KeyInput),
    Text(String),
    Preedit(String, Option<(usize, usize)>),
    Move {
        x: f32,
        y: f32,
    },
    Release,
    ContextMenu {
        x: f32,
        y: f32,
    },
    /// The primary (left) button went down at this position.
    Press {
        x: f32,
        y: f32,
    },
    /// A key went down or up. `repeat` is set for auto-repeated presses.
    Key {
        name: String,
        down: bool,
        repeat: bool,
        shift: bool,
    },
    /// The window gained or lost keyboard focus.
    Focus(bool),
}

/// deka's name for a key: lowercase characters ignoring Shift, and GPUI's names
/// for the named keys deka handles. `None` for keys deka has no name for.
pub(crate) fn key_name(key: &Key) -> Option<String> {
    let name = match key {
        Key::Character(c) => return Some(c.to_lowercase()),
        Key::Named(named) => match named {
            NamedKey::ArrowUp => "up",
            NamedKey::ArrowDown => "down",
            NamedKey::ArrowLeft => "left",
            NamedKey::ArrowRight => "right",
            NamedKey::Enter => "enter",
            NamedKey::Space => "space",
            NamedKey::Tab => "tab",
            NamedKey::Escape => "escape",
            NamedKey::Backspace => "backspace",
            NamedKey::Delete => "delete",
            NamedKey::Home => "home",
            NamedKey::End => "end",
            NamedKey::PageUp => "pageup",
            NamedKey::PageDown => "pagedown",
            NamedKey::ContextMenu => "contextmenu",
            NamedKey::F10 => "f10",
            _ => return None,
        },
        _ => return None,
    };
    Some(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::SmolStr;

    #[test]
    fn names_keys_the_way_applications_match_them() {
        let named = |k| key_name(&Key::Named(k));
        assert_eq!(named(NamedKey::Tab).as_deref(), Some("tab"));
        assert_eq!(named(NamedKey::Enter).as_deref(), Some("enter"));
        assert_eq!(named(NamedKey::Space).as_deref(), Some("space"));
        assert_eq!(named(NamedKey::Escape).as_deref(), Some("escape"));
        for (key, name) in [
            (NamedKey::ArrowUp, "up"),
            (NamedKey::ArrowDown, "down"),
            (NamedKey::ArrowLeft, "left"),
            (NamedKey::ArrowRight, "right"),
        ] {
            assert_eq!(named(key).as_deref(), Some(name));
        }
        // Shifted characters keep the key's own name; Shift travels separately.
        assert_eq!(
            key_name(&Key::Character(SmolStr::new("W"))).as_deref(),
            Some("w")
        );
        assert_eq!(
            key_name(&Key::Character(SmolStr::new("m"))).as_deref(),
            Some("m")
        );
        assert_eq!(named(NamedKey::F13), None);
    }

    #[test]
    fn the_world_moves_on_the_names_it_receives() {
        // The names above are the contract: the world must react to each one.
        let mut world = crate::world::World::new();
        world.start();
        for key in [
            NamedKey::ArrowUp,
            NamedKey::ArrowDown,
            NamedKey::ArrowLeft,
            NamedKey::ArrowRight,
        ] {
            let name = key_name(&Key::Named(key)).unwrap();
            assert!(world.key(&name, true), "{name} is a movement key");
            world.key(&name, false);
        }
        let enter = key_name(&Key::Named(NamedKey::Enter)).unwrap();
        assert!(world.key(&enter, true));
    }
}

/// The complete keyboard payload consumed by the native editor. Winit's
/// KeyEvent has private platform fields; headless drivers supply these public
/// fields to the same ingress used by the OS adapter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyInput {
    pub name: String,
    pub text: Option<String>,
    pub down: bool,
    pub shift: bool,
    pub command: bool,
    pub word: bool,
    pub control: bool,
}
#[derive(Default)]
pub struct EventLayer {
    cursor: (f32, f32),
    modifiers: winit::keyboard::ModifiersState,
}
impl EventLayer {
    pub(crate) fn translate(
        &mut self,
        event: &winit::event::WindowEvent,
        scale: f64,
    ) -> Option<Input> {
        use winit::event::{ElementState, Ime, MouseButton, WindowEvent};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        match event {
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                None
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = ((position.x / scale) as f32, (position.y / scale) as f32);
                Some(Input::Move {
                    x: self.cursor.0,
                    y: self.cursor.1,
                })
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if *state == ElementState::Pressed {
                    Some(Input::Press {
                        x: self.cursor.0,
                        y: self.cursor.1,
                    })
                } else {
                    Some(Input::Release)
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => Some(Input::ContextMenu {
                x: self.cursor.0,
                y: self.cursor.1,
            }),
            WindowEvent::KeyboardInput { event, .. } => {
                let mac = cfg!(target_os = "macos");
                Some(Input::EditKey(KeyInput {
                    name: key_name(&event.key_without_modifiers())?,
                    text: event.text.as_ref().map(ToString::to_string),
                    down: event.state == ElementState::Pressed,
                    shift: self.modifiers.shift_key(),
                    command: if mac {
                        self.modifiers.super_key()
                    } else {
                        self.modifiers.control_key()
                    },
                    word: if mac {
                        self.modifiers.alt_key()
                    } else {
                        self.modifiers.control_key()
                    },
                    control: self.modifiers.control_key(),
                }))
            }
            WindowEvent::Ime(Ime::Commit(text)) => Some(Input::Text(text.clone())),
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                Some(Input::Preedit(text.clone(), *cursor))
            }
            WindowEvent::Ime(Ime::Disabled) => Some(Input::Preedit(String::new(), None)),
            WindowEvent::Focused(focus) => {
                if !focus {
                    self.modifiers = winit::keyboard::ModifiersState::empty();
                }
                Some(Input::Focus(*focus))
            }
            _ => None,
        }
    }
}
