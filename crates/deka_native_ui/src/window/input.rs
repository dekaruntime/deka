//! Window input in deka's own terms, independent of the windowing crate.
//! Key names are the ones the GPUI adapter delivered (`"tab"`, `"up"`, `"m"`),
//! which the applications and the portfolio world already match on.
use winit::keyboard::{Key, NamedKey};

/// One input event, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input {
    /// The primary (left) button went down at this position.
    Press { x: f32, y: f32 },
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
