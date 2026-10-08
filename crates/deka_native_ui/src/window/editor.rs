//! Platform editing on Parley's shaped clusters, not byte/character estimates.
use super::input::KeyInput;
use crate::scene::{Paint, Rect, Scene};
use parley::editing::PlainEditor;
use parley::{FontContext, FontFamily, FontFamilyName, LayoutContext, StyleProperty};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
struct Snapshot {
    text: String,
    anchor: usize,
    focus: usize,
}
pub(super) struct Editor {
    pub(super) text: PlainEditor<()>,
    fonts: FontContext,
    layouts: LayoutContext<()>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    composition: Option<Snapshot>,
    pub(super) multiline: bool,
    pub(super) rect: Rect,
    pub(super) clip: Rect,
    pub(super) origin: (f32, f32),
    scroll: (f32, f32),
    pub(super) published: String,
    observed: String,
}
impl Editor {
    pub(super) fn new(value: &str, multiline: bool) -> Self {
        let (fonts, family) = crate::text::context(true);
        let mut text = PlainEditor::new(16.);
        text.edit_styles()
            .insert(StyleProperty::FontFamily(FontFamily::Single(
                FontFamilyName::Named(family.into()),
            )));
        text.set_text(value);
        Self {
            text,
            fonts,
            layouts: LayoutContext::new(),
            undo: vec![],
            redo: vec![],
            composition: None,
            multiline,
            rect: Rect::default(),
            clip: Rect::default(),
            origin: (0., 0.),
            scroll: (0., 0.),
            published: value.into(),
            observed: value.into(),
        }
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.text().to_string(),
            anchor: self.text.raw_selection().anchor().index(),
            focus: self.text.raw_selection().focus().index(),
        }
    }
    fn restore(&mut self, snapshot: &Snapshot) {
        self.text.set_text(&snapshot.text);
        self.text
            .driver(&mut self.fonts, &mut self.layouts)
            .select_byte_range(snapshot.anchor, snapshot.focus);
    }
    fn push_undo(&mut self, snapshot: Snapshot) {
        if self.undo.len() == 256 {
            self.undo.remove(0);
        }
        self.undo.push(snapshot);
        self.redo.clear();
    }
    pub(super) fn cancel(&mut self) {
        if let Some(snapshot) = self.composition.take() {
            self.restore(&snapshot);
        }
    }
    pub(super) fn compose(&mut self, value: &str, cursor: Option<(usize, usize)>) {
        if value.is_empty() {
            self.cancel();
            return;
        }
        if self.composition.is_none() {
            self.composition = Some(self.snapshot());
        }
        let cursor = cursor.filter(|(a, b)| {
            a <= b && *b <= value.len() && value.is_char_boundary(*a) && value.is_char_boundary(*b)
        });
        self.text
            .driver(&mut self.fonts, &mut self.layouts)
            .set_compose(value, cursor);
    }
    pub(super) fn insert(&mut self, value: &str) {
        let before = self.composition.take().unwrap_or_else(|| self.snapshot());
        if self.text.is_composing() {
            self.restore(&before);
        }
        let value = if self.multiline {
            value.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            value.replace(['\r', '\n'], "")
        };
        self.text
            .driver(&mut self.fonts, &mut self.layouts)
            .insert_or_replace_selection(&value);
        if self.text.text().to_string() != before.text {
            self.push_undo(before);
        }
    }
    pub(super) fn sync(&mut self, value: &str, controlled: bool) {
        if value == self.observed && (!controlled || value == self.published) {
            return;
        }
        self.observed = value.into();
        if value != self.published {
            self.cancel();
            let snapshot = self.snapshot();
            self.text.set_text(value);
            let boundary = |i: usize| {
                let mut i = i.min(value.len());
                while !value.is_char_boundary(i) {
                    i -= 1;
                }
                i
            };
            self.text
                .driver(&mut self.fonts, &mut self.layouts)
                .select_byte_range(boundary(snapshot.anchor), boundary(snapshot.focus));
            self.undo.clear();
            self.redo.clear();
            self.published = value.into();
        }
    }
    pub(super) fn place(&mut self, rect: Rect, clip: Rect) {
        self.rect = rect;
        self.clip = rect.intersection(clip).unwrap_or_default();
        self.text
            .set_width(self.multiline.then_some((rect.width - 8.).max(1.)));
        self.text.refresh_layout(&mut self.fonts, &mut self.layouts);
        let caret = self.text.ime_cursor_area();
        let visible = (rect.width - 8., rect.height - 8.);
        if caret.x1 as f32 - self.scroll.0 > visible.0 {
            self.scroll.0 = (caret.x1 as f32 - visible.0).max(0.);
        }
        if (caret.x0 as f32) < self.scroll.0 {
            self.scroll.0 = caret.x0 as f32;
        }
        if caret.y1 as f32 - self.scroll.1 > visible.1 {
            self.scroll.1 = (caret.y1 as f32 - visible.1).max(0.);
        }
        if (caret.y0 as f32) < self.scroll.1 {
            self.scroll.1 = caret.y0 as f32;
        }
        self.origin = (rect.x + 4. - self.scroll.0, rect.y + 4. - self.scroll.1);
    }
    pub(super) fn point(&mut self, x: f32, y: f32, extend: bool) {
        let mut d = self.text.driver(&mut self.fonts, &mut self.layouts);
        if extend {
            d.extend_selection_to_point(x - self.origin.0, y - self.origin.1);
        } else {
            d.move_to_point(x - self.origin.0, y - self.origin.1);
        }
    }
    pub(super) fn key(&mut self, key: &KeyInput) -> bool {
        if !key.down {
            return false;
        }
        if self.composition.is_some() {
            if key.name == "escape" {
                self.cancel();
            }
            return true;
        }
        let before = self.snapshot();
        if key.command && matches!(key.name.as_str(), "z" | "y") {
            let redo = key.shift || key.name == "y";
            let source = if redo { &mut self.redo } else { &mut self.undo };
            if let Some(snapshot) = source.pop() {
                self.restore(&snapshot);
                if redo {
                    let remaining = std::mem::take(&mut self.redo);
                    self.push_undo(before);
                    self.redo = remaining;
                } else {
                    self.redo.push(before);
                }
            }
            return true;
        }
        let mut d = self.text.driver(&mut self.fonts, &mut self.layouts);
        match (
            key.name.as_str(),
            key.shift,
            key.word,
            key.command && !key.word,
        ) {
            ("a", _, _, _) if key.command => d.select_all(),
            ("left" | "home", true, _, true) | ("home", true, false, false) => {
                d.select_to_line_start()
            }
            ("right" | "end", true, _, true) | ("end", true, false, false) => {
                d.select_to_line_end()
            }
            ("left" | "home", false, _, true) | ("home", false, false, false) => {
                d.move_to_line_start()
            }
            ("right" | "end", false, _, true) | ("end", false, false, false) => {
                d.move_to_line_end()
            }
            ("up", true, _, true) => d.select_to_text_start(),
            ("down", true, _, true) => d.select_to_text_end(),
            ("up", false, _, true) => d.move_to_text_start(),
            ("down", false, _, true) => d.move_to_text_end(),
            ("left", true, true, _) => d.select_word_left(),
            ("right", true, true, _) => d.select_word_right(),
            ("left", false, true, _) => d.move_word_left(),
            ("right", false, true, _) => d.move_word_right(),
            ("left", true, false, _) => d.select_left(),
            ("right", true, false, _) => d.select_right(),
            ("left", false, false, _) => d.move_left(),
            ("right", false, false, _) => d.move_right(),
            ("up", true, _, _) => d.select_up(),
            ("down", true, _, _) => d.select_down(),
            ("up", false, _, _) => d.move_up(),
            ("down", false, _, _) => d.move_down(),
            ("backspace", _, true, _) => d.backdelete_word(),
            ("delete", _, true, _) => d.delete_word(),
            ("backspace", _, false, _) => {
                if d.editor.raw_selection().is_collapsed() {
                    let index = d.editor.raw_selection().focus().index();
                    if let Some(cluster) = d.editor.raw_text()[..index].graphemes(true).next_back()
                    {
                        d.delete_bytes_before_selection(
                            std::num::NonZeroUsize::new(cluster.len()).unwrap(),
                        );
                    }
                } else {
                    d.delete_selection();
                }
            }
            ("delete", _, false, _) => d.delete(),
            ("enter", _, _, _) if self.multiline => d.insert_or_replace_selection("\n"),
            ("escape", _, _, _) => d.collapse_selection(),
            _ => {
                if !key.command
                    && !key.control
                    && let Some(text) = &key.text
                    && !text.chars().any(char::is_control)
                {
                    d.insert_or_replace_selection(text);
                } else {
                    return false;
                }
            }
        }
        if self.text.text().to_string() != before.text {
            self.push_undo(before);
        }
        true
    }
    pub(super) fn delete_selected(&mut self) {
        self.insert("");
    }
    pub(super) fn decoration(&self, scene: &mut Scene, focused: bool) {
        let draw = |scene: &mut Scene, b: parley::BoundingBox, color, underline| {
            let mut rect = Rect {
                x: self.origin.0 + b.x0 as f32,
                y: self.origin.1 + b.y0 as f32,
                width: (b.x1 - b.x0) as f32,
                height: (b.y1 - b.y0) as f32,
            };
            if underline {
                rect.y += rect.height - 1.;
                rect.height = 1.;
            }
            scene.paint.push(Paint {
                rect,
                clip: self.clip,
                color,
                radius: 0.,
                image: None,
                opacity: 1.,
            });
        };
        if focused {
            for (b, _) in self.text.selection_geometry() {
                draw(scene, b, 0xc5d9f5, false);
            }
            if self.text.raw_selection().is_collapsed()
                && let Some(b) = self.text.cursor_geometry(1.5)
            {
                draw(scene, b, 0x1a1611, false);
            }
        }
        if let Some(range) = self.text.raw_compose()
            && let Some(layout) = self.text.try_layout()
        {
            let a = parley::editing::Cursor::from_byte_index(
                layout,
                range.start,
                parley::layout::Affinity::Downstream,
            );
            let b = parley::editing::Cursor::from_byte_index(
                layout,
                range.end,
                parley::layout::Affinity::Upstream,
            );
            for (rect, _) in parley::editing::Selection::new(a, b).geometry(layout) {
                draw(scene, rect, 0x1a1611, true);
            }
        }
    }
    pub(super) fn ime_area(&self) -> Rect {
        let b = self.text.ime_cursor_area();
        Rect {
            x: self.origin.0 + b.x0 as f32,
            y: self.origin.1 + b.y0 as f32,
            width: (b.x1 - b.x0) as f32,
            height: (b.y1 - b.y0) as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redo_caps_undo_and_preserves_remaining_redos() {
        let mut editor = Editor::new("a", false);
        let snapshot = editor.snapshot();
        editor.undo = vec![snapshot.clone(); 256];
        editor.redo = vec![snapshot; 2];
        editor.key(&KeyInput {
            name: "y".into(),
            command: true,
            down: true,
            ..Default::default()
        });
        assert_eq!(editor.undo.len(), 256);
        assert_eq!(editor.redo.len(), 1);
    }
}
