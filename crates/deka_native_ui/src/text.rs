//! One text layout path is used for measurement and painting, in logical pixels.
use fontdue::{
    Font,
    layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle},
};

pub(crate) fn layout(font: &Font, text: &str, size: f32, width: Option<f32>) -> Layout {
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        max_width: width.map(|w| w.max(0.)),
        ..Default::default()
    });
    layout.append(&[font], &TextStyle::new(text, size, 0));
    layout
}
pub(crate) fn width(font: &Font, layout: &Layout, size: f32) -> f32 {
    layout
        .glyphs()
        .iter()
        .map(|g| {
            let m = font.metrics(g.parent, size);
            g.x - m.xmin as f32 + m.advance_width.ceil()
        })
        .fold(0., f32::max)
}
pub(crate) fn min_width(font: &Font, text: &str, size: f32) -> f32 {
    text.split_whitespace()
        .map(|word| width(font, &layout(font, word, size, None), size))
        .fold(0., f32::max)
}
