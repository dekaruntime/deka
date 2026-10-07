//! One text path for measuring and painting, in logical pixels.
//! parley shapes, breaks lines and falls back to system fonts (fontique); swash rasterises.
//! Shaped layouts and rasterised glyphs are cached across frames.
use crate::scene::{GlyphImage, Rect};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, Layout, LayoutContext,
    PositionedLayoutItem, StyleProperty,
    fontique::{Blob, Collection, CollectionOptions},
};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, OnceLock},
};
use swash::{
    FontRef,
    scale::{Render, ScaleContext, Source, StrikeWith, image::Content},
    zeno::{Format, Vector},
};

const BUNDLED: &[u8] = include_bytes!("../assets/AtkinsonHyperlegible-Regular.ttf");
/// Horizontal subpixel positions per pixel. Vertical positions snap to the device pixel grid.
const SUBPIXEL: f32 = 4.;
/// Entries unused for this many renders are evicted once a cache passes its soft limit.
const KEEP_RENDERS: u64 = 120;
/// CoreText thickens outlines when it smooths text. Emboldening by 1% of the pixel size matches
/// its ink coverage of this face within 1% at 1x and 2x, so text weighs the same on every platform.
const DILATION: f32 = 0.01;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u64,
    glyph: u32,
    /// Device pixels per em, as f32 bits.
    size: u32,
    subpixel: u8,
    coords: u64,
}
struct Raster {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    /// True for colour glyphs (emoji): `data` is RGBA. Otherwise `data` is an alpha mask.
    color: bool,
    data: Vec<u8>,
}
/// One shaped run: its font and glyph ids with positions relative to the layout origin.
struct GlyphRun {
    data: Blob<u8>,
    index: u32,
    font: u64,
    size: f32,
    coords: Vec<i16>,
    coords_hash: u64,
    glyphs: Vec<(u32, f32, f32)>,
}
struct Cached<T> {
    value: T,
    used: u64,
}

fn bundled_collection(system_fonts: bool) -> (Collection, String) {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts,
    });
    let registered = collection.register_fonts(Blob::new(Arc::new(BUNDLED)), None);
    let family = registered
        .first()
        .and_then(|(id, _)| collection.family_name(*id))
        .expect("bundled font registers a family")
        .to_owned();
    (collection, family)
}
/// The bundled face is registered once per process (and the system scan runs at most once).
/// Clones share the font data, so every renderer produces the same glyphs and image ids.
fn context(system_fonts: bool) -> (FontContext, String) {
    static BUNDLED_ONLY: OnceLock<(Collection, String)> = OnceLock::new();
    static WITH_SYSTEM: OnceLock<(Collection, String)> = OnceLock::new();
    let cell = if system_fonts {
        &WITH_SYSTEM
    } else {
        &BUNDLED_ONLY
    };
    let (collection, family) = cell.get_or_init(|| bundled_collection(system_fonts));
    (
        FontContext {
            collection: collection.clone(),
            source_cache: Default::default(),
        },
        family.clone(),
    )
}
/// Whether the bundled face maps every character. Only text it cannot draw needs the system
/// font scan, which costs tens of milliseconds; Latin-only applications never pay it.
fn bundled_covers(text: &str) -> bool {
    static CHARMAP: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    let Some(font) = CHARMAP.get_or_init(|| FontRef::from_index(BUNDLED, 0)) else {
        return false;
    };
    let charmap = font.charmap();
    text.chars().all(|c| c.is_control() || charmap.map(c) != 0)
}

/// Register the bundled face and load its character map on a thread, so the
/// window's first layout finds them ready (`context` blocks on the same cell
/// if it gets there first). The system font scan stays lazy.
#[cfg(feature = "gpu")]
pub(crate) fn warm_on_thread() {
    let _ = std::thread::Builder::new()
        .name("deka-fonts".into())
        .spawn(|| {
            let _ = context(false);
            bundled_covers("");
        });
}

pub(crate) struct Text {
    fonts: FontContext,
    /// Created on the first string the bundled face cannot draw (Japanese, emoji, ...).
    system: Option<FontContext>,
    layouts: LayoutContext<()>,
    family: String,
    shaped: HashMap<(Box<str>, u32), Cached<Layout<()>>>,
    scaler: ScaleContext,
    glyphs: HashMap<GlyphKey, Cached<Option<Arc<Raster>>>>,
    /// Blob ids are per process and per collection; scene image ids must not be.
    font_ids: HashMap<u64, u64>,
    render: u64,
    #[cfg(test)]
    rasterised: usize,
}
impl Text {
    pub(crate) fn new() -> Self {
        let (fonts, family) = context(false);
        Self {
            fonts,
            system: None,
            family,
            layouts: LayoutContext::new(),
            shaped: HashMap::new(),
            scaler: ScaleContext::new(),
            glyphs: HashMap::new(),
            font_ids: HashMap::new(),
            render: 0,
            #[cfg(test)]
            rasterised: 0,
        }
    }

    /// Mark the end of one scene render and drop cache entries that stopped being used.
    pub(crate) fn end_render(&mut self) {
        self.render += 1;
        let oldest = self.render.saturating_sub(KEEP_RENDERS);
        if self.shaped.len() > 512 {
            self.shaped.retain(|_, c| c.used >= oldest);
        }
        if self.glyphs.len() > 4096 {
            self.glyphs.retain(|_, c| c.used >= oldest);
        }
    }

    /// Shape once per (text, size); every later call only re-breaks lines.
    fn layout(&mut self, text: &str, size: f32, width: Option<f32>) -> &Layout<()> {
        let key = (Box::<str>::from(text), size.to_bits());
        let render = self.render;
        let entry = self.shaped.entry(key).or_insert_with(|| {
            let fonts = if bundled_covers(text) {
                &mut self.fonts
            } else {
                self.system.get_or_insert_with(|| context(true).0)
            };
            let mut builder = self.layouts.ranged_builder(fonts, text, 1., true);
            builder.push_default(StyleProperty::FontFamily(FontFamily::Single(
                FontFamilyName::Named(self.family.as_str().into()),
            )));
            builder.push_default(StyleProperty::FontSize(size));
            Cached {
                value: builder.build(text),
                used: render,
            }
        });
        entry.used = render;
        let layout = &mut entry.value;
        layout.break_all_lines(width.map(|w| w.max(0.)));
        layout.align(Alignment::Start, AlignmentOptions::default());
        layout
    }

    /// Size of `text` broken at `width` (`None`: only hard breaks). Width is rounded up to whole
    /// pixels so a box that taffy rounds back to the measured width re-breaks identically.
    pub(crate) fn measure(&mut self, text: &str, size: f32, width: Option<f32>) -> (f32, f32) {
        let layout = self.layout(text, size, width);
        (layout.width().ceil(), layout.height())
    }

    /// Narrowest width that takes every soft break opportunity.
    pub(crate) fn min_width(&mut self, text: &str, size: f32) -> f32 {
        self.layout(text, size, None)
            .calculate_content_widths()
            .min
            .ceil()
    }

    #[cfg(test)]
    pub(crate) fn line_count(&mut self, text: &str, size: f32, width: Option<f32>) -> usize {
        self.layout(text, size, width).len()
    }

    fn runs(&mut self, text: &str, size: f32, width: Option<f32>) -> Vec<GlyphRun> {
        let mut runs = vec![];
        for line in self.layout(text, size, width).lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let r = run.run();
                let mut coords_hash = std::collections::hash_map::DefaultHasher::new();
                r.normalized_coords().hash(&mut coords_hash);
                runs.push(GlyphRun {
                    data: r.font().data.clone(),
                    index: r.font().index,
                    font: 0,
                    size: r.font_size(),
                    coords: r.normalized_coords().to_vec(),
                    coords_hash: coords_hash.finish(),
                    glyphs: run.positioned_glyphs().map(|g| (g.id, g.x, g.y)).collect(),
                });
            }
        }
        for run in &mut runs {
            run.font = self.font_id(&run.data, run.index);
        }
        runs
    }

    fn font_id(&mut self, data: &Blob<u8>, index: u32) -> u64 {
        *self.font_ids.entry(data.id()).or_insert_with(|| {
            // The table directory (tags, checksums, offsets) identifies a font file.
            let bytes = data.data();
            // Hash explicit bytes, not usize/slice Hash (whose length prefix
            // differs on wasm32). The same face has the same scene image ID.
            (bytes.len() as u64)
                .to_le_bytes()
                .into_iter()
                .chain(bytes[..bytes.len().min(1024)].iter().copied())
                .fold(0xcbf29ce484222325u64, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
                })
        }) ^ u64::from(index)
    }

    fn raster(
        &mut self,
        key: GlyphKey,
        data: &Blob<u8>,
        index: u32,
        coords: &[i16],
    ) -> Option<Arc<Raster>> {
        let render = self.render;
        if let Some(cached) = self.glyphs.get_mut(&key) {
            cached.used = render;
            return cached.value.clone();
        }
        #[cfg(test)]
        {
            self.rasterised += 1;
        }
        let raster = FontRef::from_index(data.data(), index as usize).and_then(|font| {
            let mut scaler = self
                .scaler
                .builder(font)
                .size(f32::from_bits(key.size))
                .normalized_coords(coords)
                .build();
            let image = Render::new(&[
                Source::ColorOutline(0),
                Source::ColorBitmap(StrikeWith::BestFit),
                Source::Outline,
            ])
            .format(Format::Alpha)
            .embolden(f32::from_bits(key.size) * DILATION)
            .offset(Vector::new(f32::from(key.subpixel) / SUBPIXEL, 0.))
            .render(&mut scaler, u16::try_from(key.glyph).ok()?)?;
            (image.placement.width > 0 && image.placement.height > 0).then(|| {
                Arc::new(Raster {
                    left: image.placement.left,
                    top: image.placement.top,
                    width: image.placement.width as usize,
                    height: image.placement.height as usize,
                    color: image.content == Content::Color,
                    data: image.data,
                })
            })
        });
        self.glyphs.insert(
            key,
            Cached {
                value: raster.clone(),
                used: render,
            },
        );
        raster
    }

    /// Glyph quads for `text` laid out at logical `origin`, rasterised at device `scale`.
    /// Each glyph's image is added to `images` once per scene, tinted with `color`.
    #[allow(clippy::too_many_arguments)] // One text run: string, style, box and the scene's image table.
    pub(crate) fn glyphs(
        &mut self,
        text: &str,
        size: f32,
        width: Option<f32>,
        origin: (f32, f32),
        scale: f32,
        color: u32,
        images: &mut HashMap<String, GlyphImage>,
    ) -> Vec<(Rect, String)> {
        let mut out = vec![];
        for run in self.runs(text, size, width) {
            let font = run.font;
            for &(glyph, x, y) in &run.glyphs {
                let x = (origin.0 + x) * scale;
                let y = ((origin.1 + y) * scale).round();
                let mut left = x.floor();
                let mut subpixel = ((x - left) * SUBPIXEL).round();
                if subpixel >= SUBPIXEL {
                    left += 1.;
                    subpixel = 0.;
                }
                let key = GlyphKey {
                    font,
                    glyph,
                    size: (run.size * scale).to_bits(),
                    subpixel: subpixel as u8,
                    coords: run.coords_hash,
                };
                let Some(raster) = self.raster(key, &run.data, run.index, &run.coords) else {
                    continue;
                };
                let id = if raster.color {
                    format!("t{font:x}-{glyph}-{:x}-{}", key.size, key.subpixel)
                } else {
                    format!(
                        "t{font:x}-{glyph}-{:x}-{}-{color:06x}",
                        key.size, key.subpixel
                    )
                };
                images.entry(id.clone()).or_insert_with(|| GlyphImage {
                    id: id.clone(),
                    width: raster.width,
                    height: raster.height,
                    rgba: if raster.color {
                        raster.data.clone()
                    } else {
                        raster
                            .data
                            .iter()
                            .flat_map(|a| {
                                [(color >> 16) as u8, (color >> 8) as u8, color as u8, *a]
                            })
                            .collect()
                    },
                });
                out.push((
                    Rect {
                        x: (left + raster.left as f32) / scale,
                        y: (y - raster.top as f32) / scale,
                        width: raster.width as f32 / scale,
                        height: raster.height as f32 / scale,
                    },
                    id,
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PARAGRAPH: &str = "Deka draws its own text: shaping, line breaking and fallback all happen before the GPU sees a single glyph.";

    /// Width and line height from the font's own tables, independent of parley.
    fn font_metrics(text: &str, size: f32) -> (f32, f32) {
        let font = FontRef::from_index(BUNDLED, 0).unwrap();
        let charmap = font.charmap();
        let glyphs = font.glyph_metrics(&[]).scale(size);
        let advance = text
            .chars()
            .map(|c| glyphs.advance_width(charmap.map(c)))
            .sum();
        let m = font.metrics(&[]).scale(size);
        (advance, m.ascent + m.descent + m.leading)
    }

    #[test]
    fn measures_known_strings_with_the_bundled_face() {
        let mut text = Text::new();
        for size in [11., 12., 14., 16., 24.] {
            let (width, height) = text.measure("Hamburgefonstiv", size, None);
            let (advance, line) = font_metrics("Hamburgefonstiv", size);
            assert!(
                (width - advance.ceil()).abs() <= 1.,
                "{size}px: measured {width}, font advances {advance}"
            );
            assert!((height - line).abs() < 0.01, "{size}px: {height} vs {line}");
        }
        assert_eq!(text.measure("Hamburgefonstiv", 16., None).0, 120.);
        assert_eq!(text.measure("", 16., None).0, 0.);
    }

    #[test]
    fn wraps_at_the_box_width_and_only_at_break_opportunities() {
        let mut text = Text::new();
        let line = font_metrics("", 14.).1;
        assert_eq!(text.line_count(PARAGRAPH, 14., None), 1);
        for (width, lines) in [(300., 3), (200., 4), (120., 6)] {
            assert_eq!(
                text.line_count(PARAGRAPH, 14., Some(width)),
                lines,
                "at {width}px"
            );
            let (w, h) = text.measure(PARAGRAPH, 14., Some(width));
            assert!(w <= width, "{w} overflows {width}");
            assert!(
                (h - lines as f32 * line).abs() < 0.01,
                "{h} for {lines} lines"
            );
        }
        // The narrowest box is the longest word, never a break inside it.
        assert_eq!(
            text.min_width(PARAGRAPH, 14.),
            text.measure("breaking", 14., None).0
        );
        assert_eq!(text.line_count("one\ntwo", 14., None), 2);
    }

    #[test]
    fn rasterises_each_glyph_once_across_frames_and_colours() {
        let mut text = Text::new();
        let draw = |text: &mut Text, scale: f32, color: u32| {
            let mut images = HashMap::new();
            let glyphs = text.glyphs(
                "Cached text",
                16.,
                None,
                (3.3, 7.),
                scale,
                color,
                &mut images,
            );
            text.end_render();
            (glyphs.len(), images.len())
        };
        let (glyphs, images) = draw(&mut text, 2., 0x112233);
        assert_eq!(glyphs, 10, "every visible glyph is painted");
        assert!(images > 0 && text.rasterised >= images);
        let first = text.rasterised;
        for _ in 0..3 {
            assert_eq!(draw(&mut text, 2., 0x112233), (glyphs, images));
        }
        draw(&mut text, 2., 0xff0000);
        assert_eq!(text.rasterised, first, "frames and colours reuse rasters");
        draw(&mut text, 1., 0x112233);
        assert!(
            text.rasterised > first,
            "a new device scale rasterises again"
        );
    }

    #[test]
    fn rasterises_at_device_scale_with_the_same_logical_geometry() {
        let mut text = Text::new();
        let mut images = HashMap::new();
        let one = text.glyphs("H", 16., None, (0., 0.), 1., 0, &mut images);
        let two = text.glyphs("H", 16., None, (0., 0.), 2., 0, &mut images);
        let (r1, r2) = (one[0].0, two[0].0);
        let (i1, i2) = (&images[&one[0].1], &images[&two[0].1]);
        assert!(
            i2.height.abs_diff(2 * i1.height) <= 2,
            "{} vs {}",
            i1.height,
            i2.height
        );
        assert!(
            i2.width.abs_diff(2 * i1.width) <= 2,
            "{} vs {}",
            i1.width,
            i2.width
        );
        assert!((r1.y - r2.y).abs() <= 1. && (r1.height - r2.height).abs() <= 1.);
        // Glyph tops and bottoms sit on whole device pixels.
        for (r, scale) in [(r1, 1.), (r2, 2.)] {
            assert_eq!(r.y * scale, (r.y * scale).round());
            assert_eq!(r.height * scale, (r.height * scale).round());
        }
    }

    /// macOS always ships Japanese and colour emoji faces; other hosts may not.
    #[cfg(target_os = "macos")]
    #[test]
    fn falls_back_to_system_fonts_for_japanese_and_emoji() {
        let mut text = Text::new();
        let bundled = text.runs("A", 16., None)[0].font;
        let runs = text.runs("日本語", 16., None);
        let glyphs: Vec<_> = runs.iter().flat_map(|r| &r.glyphs).collect();
        assert_eq!(glyphs.len(), 3);
        assert!(glyphs.iter().all(|g| g.0 != 0), "no .notdef boxes");
        assert!(
            runs.iter().all(|r| r.font != bundled),
            "drawn by a system face"
        );
        let mut images = HashMap::new();
        let painted = text.glyphs("日本語 🎉", 16., None, (0., 0.), 2., 0x111111, &mut images);
        assert_eq!(painted.len(), 4);
        let emoji = &images[&painted[3].1];
        let colours: std::collections::HashSet<_> = emoji
            .rgba
            .chunks_exact(4)
            .filter(|p| p[3] == 255)
            .map(|p| [p[0], p[1], p[2]])
            .collect();
        assert!(
            colours.len() > 10,
            "emoji keeps its own colours ({} found)",
            colours.len()
        );
    }
}
