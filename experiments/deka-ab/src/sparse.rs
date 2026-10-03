//! Backends C and D: vello's sparse-strips renderers drawing the same
//! deka_native_ui Scene in a winit window.
//! * C `ab-hybrid`: vello_gpu (formerly vello_hybrid): CPU strips, GPU raster
//!   with vertex/fragment shaders, rendering straight into the wgpu surface.
//! * D `ab-cpu`: vello_cpu (SIMD + threads) presented with softbuffer. No wgpu.
//!
//! vello_cpu's RenderContext and vello_gpu's Scene expose the same stateful,
//! canvas-shaped API; `Sparse` is the subset deka needs, implemented for both
//! by one macro.

#[path = "../../vello-gpui/src/graph.rs"]
#[allow(dead_code, reason = "shared with the phase-1 crate, which also uses the Hairball layout")]
mod graph;

#[cfg(feature = "cpu-backend")]
pub mod cpu;
#[cfg(feature = "hybrid-backend")]
pub mod hybrid;

/// What graph.rs (and this module) draw with: vello_common's kurbo/peniko.
pub mod gfx {
    #[cfg(feature = "cpu-backend")]
    pub use vello_cpu::{Glyph, PaintType, kurbo, peniko};
    #[cfg(all(feature = "hybrid-backend", not(feature = "cpu-backend")))]
    pub use self::gpu::*;
    #[cfg(all(feature = "hybrid-backend", not(feature = "cpu-backend")))]
    mod gpu {
        pub use glifo::Glyph;
        pub use vello_gpu::{PaintType, kurbo, peniko};
    }
}

use crate::common::{self, App as AppKind, Args, Protocol, Step, WINDOW_H, WINDOW_W};
use deka_native_ui::scene::{GlyphImage, Scene as DekaScene};
use deka_native_ui::{Host, world::World};
use gfx::kurbo::{Affine, BezPath, Circle, Join, Line, Point, Rect, RoundedRect, Shape, Stroke};
use gfx::peniko::{BlendMode, Blob, Brush, Color, Compose, Fill, FontData, Gradient, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageSampler, Mix};
use gfx::{Glyph, PaintType};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

pub type ImageSource = vello_common_image::ImageSource;
pub use vello_common_image::filter_effects;
mod vello_common_image {
    #[cfg(feature = "cpu-backend")]
    pub use vello_cpu::{ImageSource, filter_effects};
    #[cfg(all(feature = "hybrid-backend", not(feature = "cpu-backend")))]
    pub use vello_gpu::{ImageSource, filter_effects};
}

const TOLERANCE: f64 = 0.1;

pub fn rgb(c: u32, alpha: f32) -> Color {
    Color::from_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, (alpha.clamp(0., 1.) * 255.).round() as u8)
}

/// The deka-relevant subset of the sparse-strips drawing API.
pub trait Sparse {
    type Res;
    fn set_transform(&mut self, t: Affine);
    fn set_paint(&mut self, p: PaintType);
    fn set_paint_transform(&mut self, t: Affine);
    fn set_fill_rule(&mut self, f: Fill);
    fn set_stroke(&mut self, s: Stroke);
    fn fill_path(&mut self, p: &BezPath);
    fn stroke_path(&mut self, p: &BezPath);
    /// Fill a kurbo shape (backends without a shape API build a `BezPath`).
    fn fill_shape(&mut self, shape: &impl Shape) {
        self.fill_path(&shape.to_path(TOLERANCE));
    }
    /// Stroke a kurbo shape (backends without a shape API build a `BezPath`).
    fn stroke_shape(&mut self, shape: &impl Shape) {
        self.stroke_path(&shape.to_path(TOLERANCE));
    }
    fn fill_rect(&mut self, r: &Rect);
    fn push_layer(&mut self, clip: Option<&BezPath>, blend: Option<BlendMode>, opacity: Option<f32>);
    fn pop_layer(&mut self);
    fn push_filter_layer(&mut self, f: filter_effects::Filter);
    fn fill_blurred_rounded_rect(&mut self, r: &Rect, radius: f32, std_dev: f32);
    fn glyphs(&mut self, res: &mut Self::Res, font: &FontData, size: f32, glyphs: &[Glyph]) -> bool;
}

macro_rules! impl_sparse {
    ($ty:ty, $res:ty $(, $extra:item)*) => {
        impl Sparse for $ty {
            type Res = $res;
            $($extra)*
            fn set_transform(&mut self, t: Affine) {
                <$ty>::set_transform(self, t)
            }
            fn set_paint(&mut self, p: PaintType) {
                <$ty>::set_paint(self, p)
            }
            fn set_paint_transform(&mut self, t: Affine) {
                <$ty>::set_paint_transform(self, t)
            }
            fn set_fill_rule(&mut self, f: Fill) {
                <$ty>::set_fill_rule(self, f)
            }
            fn set_stroke(&mut self, s: Stroke) {
                <$ty>::set_stroke(self, s)
            }
            fn fill_path(&mut self, p: &BezPath) {
                <$ty>::fill_path(self, p)
            }
            fn stroke_path(&mut self, p: &BezPath) {
                <$ty>::stroke_path(self, p)
            }
            fn fill_rect(&mut self, r: &Rect) {
                <$ty>::fill_rect(self, r)
            }
            fn push_layer(&mut self, clip: Option<&BezPath>, blend: Option<BlendMode>, opacity: Option<f32>) {
                <$ty>::push_layer(self, clip, blend, opacity, None, None)
            }
            fn pop_layer(&mut self) {
                <$ty>::pop_layer(self)
            }
            fn push_filter_layer(&mut self, f: filter_effects::Filter) {
                <$ty>::push_filter_layer(self, f)
            }
            fn fill_blurred_rounded_rect(&mut self, r: &Rect, radius: f32, std_dev: f32) {
                <$ty>::fill_blurred_rounded_rect(self, r, radius, std_dev, false)
            }
            fn glyphs(&mut self, res: &mut $res, font: &FontData, size: f32, glyphs: &[Glyph]) -> bool {
                self.glyph_run(res, font).font_size(size).hint(true).fill_glyphs(glyphs.iter().copied()).is_ok()
            }
        }
    };
}
#[cfg(feature = "cpu-backend")]
impl_sparse!(vello_cpu::RenderContext, vello_cpu::Resources);
#[cfg(feature = "hybrid-backend")]
impl_sparse!(
    vello_gpu::Scene,
    vello_gpu::Resources,
    // The fork's shape API: no intermediate BezPath (unless the verify
    // control asks for the stock `fill_path(&to_path())` route).
    fn fill_shape(&mut self, shape: &impl Shape) {
        if SHAPE_API.load(std::sync::atomic::Ordering::Relaxed) {
            vello_gpu::Scene::fill_shape(self, shape)
        } else {
            vello_gpu::Scene::fill_path(self, &shape.to_path(TOLERANCE))
        }
    },
    fn stroke_shape(&mut self, shape: &impl Shape) {
        if SHAPE_API.load(std::sync::atomic::Ordering::Relaxed) {
            vello_gpu::Scene::stroke_shape(self, shape)
        } else {
            vello_gpu::Scene::stroke_path(self, &shape.to_path(TOLERANCE))
        }
    }
);

/// Whether the graph painter uses the backend's shape API (the fork's
/// `fill_shape`) or the stock `fill_path(&shape.to_path())`.
pub static SHAPE_API: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Straight RGBA glyph/sprite bitmap -> sparse ImageSource (pixmap form).
pub fn pixmap_source(g: &GlyphImage) -> ImageSource {
    ImageSource::from_peniko_image_data(&ImageData {
        data: Blob::new(Arc::new(g.rgba.clone())),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: g.width as u32,
        height: g.height as u32,
    })
}

/// deka Scene -> sparse drawing calls. `images` maps deka image ids to
/// sources the backend can draw (pixmaps for CPU, uploaded ids for GPU).
/// Clip layers are shared across consecutive paints (see phase 2).
pub fn encode_deka<S: Sparse>(ctx: &mut S, deka: &DekaScene, scale: f64, images: &HashMap<String, ImageSource>) {
    let s = Affine::scale(scale);
    ctx.set_transform(s);
    ctx.set_paint_transform(Affine::IDENTITY);
    ctx.set_fill_rule(Fill::NonZero);
    ctx.set_paint(rgb(deka.background, 1.).into());
    ctx.fill_rect(&Rect::new(0., 0., deka.width as f64, deka.height as f64));
    let mut active_clip: Option<Rect> = None;
    for p in &deka.paint {
        if p.opacity <= 0. {
            continue;
        }
        let r = Rect::new(p.rect.x as f64, p.rect.y as f64, (p.rect.x + p.rect.width) as f64, (p.rect.y + p.rect.height) as f64);
        let c = Rect::new(p.clip.x as f64, p.clip.y as f64, (p.clip.x + p.clip.width) as f64, (p.clip.y + p.clip.height) as f64);
        let inside = r.x0 >= c.x0 && r.y0 >= c.y0 && r.x1 <= c.x1 && r.y1 <= c.y1;
        if !inside && active_clip != Some(c) {
            if active_clip.is_some() {
                ctx.pop_layer();
            }
            ctx.set_transform(s);
            ctx.push_layer(Some(&c.to_path(TOLERANCE)), None, None);
            active_clip = Some(c);
        } else if inside
            && let Some(a) = active_clip
            && !(r.x0 >= a.x0 && r.y0 >= a.y0 && r.x1 <= a.x1 && r.y1 <= a.y1)
        {
            ctx.pop_layer();
            active_clip = None;
        }
        if let Some(id) = &p.image {
            let (Some(src), Some(g)) = (images.get(id), deka.images.iter().find(|g| &g.id == id)) else { continue };
            // Device-pixel snapped, like the vello backend.
            let (dx, dy) = ((r.x0 * scale).round(), (r.y0 * scale).round());
            let (dw, dh) = (r.width() * scale, r.height() * scale);
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(Brush::Image(ImageBrush { image: src.clone(), sampler: ImageSampler::default() }.with_alpha(p.opacity)));
            ctx.set_paint_transform(Affine::translate((dx, dy)) * Affine::scale_non_uniform(dw / g.width.max(1) as f64, dh / g.height.max(1) as f64));
            ctx.fill_rect(&Rect::new(dx, dy, dx + dw, dy + dh));
            ctx.set_paint_transform(Affine::IDENTITY);
        } else {
            ctx.set_transform(s);
            ctx.set_paint(rgb(p.color, p.opacity).into());
            if p.radius > 0. {
                ctx.fill_shape(&RoundedRect::from_rect(r, p.radius as f64));
            } else {
                ctx.fill_rect(&r);
            }
        }
    }
    if active_clip.is_some() {
        ctx.pop_layer();
    }
}

/// graph.rs Painter over a Sparse context (phase-1 canvas scenes).
pub struct SparsePainter<'a, S: Sparse> {
    pub ctx: &'a mut S,
}
impl<S: Sparse> graph::Painter for SparsePainter<'_, S> {
    fn fill(&mut self, shape: &impl Shape, color: Color) {
        self.ctx.set_paint(color.into());
        self.ctx.fill_shape(shape);
    }
    fn stroke(&mut self, shape: &impl Shape, style: &Stroke, color: Color) {
        self.ctx.set_paint(color.into());
        self.ctx.set_stroke(style.clone());
        self.ctx.stroke_shape(shape);
    }
}

pub fn encode_graph<S: Sparse>(ctx: &mut S, g: &graph::Graph, scale: f64, w: f64, h: f64) {
    ctx.set_transform(Affine::IDENTITY);
    ctx.set_paint(graph::BACKGROUND.into());
    ctx.fill_rect(&Rect::new(0., 0., w, h));
    ctx.set_transform(Affine::scale(scale));
    g.paint(&mut SparsePainter { ctx });
}

// ---------------------------------------------------------------------------
// Canvas 2D coverage sheet (the phase-1 tiles, through the sparse API)

pub const COVERAGE: (u32, u32) = (1600, 1200);
const TILE: (f64, f64) = (400.0, 300.0);
const INK: Color = Color::from_rgba8(0x22, 0x22, 0x22, 0xff);
const RED: Color = Color::from_rgba8(0xe6, 0x39, 0x46, 0xff);
const BLUE: Color = Color::from_rgba8(0x1d, 0x35, 0x57, 0xff);
const TEAL: Color = Color::from_rgba8(0x2a, 0x9d, 0x8f, 0xff);
const GOLD: Color = Color::from_rgba8(0xe9, 0xc4, 0x6a, 0xff);

pub const BLEND_MODES: [(&str, Mix, Compose); 26] = [
    ("source-over", Mix::Normal, Compose::SrcOver),
    ("source-in", Mix::Normal, Compose::SrcIn),
    ("source-out", Mix::Normal, Compose::SrcOut),
    ("source-atop", Mix::Normal, Compose::SrcAtop),
    ("destination-over", Mix::Normal, Compose::DestOver),
    ("destination-in", Mix::Normal, Compose::DestIn),
    ("destination-out", Mix::Normal, Compose::DestOut),
    ("destination-atop", Mix::Normal, Compose::DestAtop),
    ("lighter", Mix::Normal, Compose::Plus),
    ("copy", Mix::Normal, Compose::Copy),
    ("xor", Mix::Normal, Compose::Xor),
    ("multiply", Mix::Multiply, Compose::SrcOver),
    ("screen", Mix::Screen, Compose::SrcOver),
    ("overlay", Mix::Overlay, Compose::SrcOver),
    ("darken", Mix::Darken, Compose::SrcOver),
    ("lighten", Mix::Lighten, Compose::SrcOver),
    ("color-dodge", Mix::ColorDodge, Compose::SrcOver),
    ("color-burn", Mix::ColorBurn, Compose::SrcOver),
    ("hard-light", Mix::HardLight, Compose::SrcOver),
    ("soft-light", Mix::SoftLight, Compose::SrcOver),
    ("difference", Mix::Difference, Compose::SrcOver),
    ("exclusion", Mix::Exclusion, Compose::SrcOver),
    ("hue", Mix::Hue, Compose::SrcOver),
    ("saturation", Mix::Saturation, Compose::SrcOver),
    ("color", Mix::Color, Compose::SrcOver),
    ("luminosity", Mix::Luminosity, Compose::SrcOver),
];

fn tile(i: usize) -> (f64, f64) {
    ((i % 4) as f64 * TILE.0, (i / 4) as f64 * TILE.1)
}

fn path(points: &[(f64, f64)], close: bool) -> BezPath {
    let mut p = BezPath::new();
    for (i, pt) in points.iter().enumerate() {
        if i == 0 { p.move_to(*pt) } else { p.line_to(*pt) }
    }
    if close {
        p.close_path();
    }
    p
}

fn star(c: Point, r_out: f64, r_in: f64, n: usize) -> BezPath {
    let pts: Vec<_> = (0..n * 2)
        .map(|i| {
            let r = if i % 2 == 0 { r_out } else { r_in };
            let a = std::f64::consts::PI * i as f64 / n as f64 - std::f64::consts::FRAC_PI_2;
            (c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    path(&pts, true)
}

fn pentagram(c: Point, r: f64) -> BezPath {
    let pts: Vec<_> = (0..5)
        .map(|k| {
            let a = std::f64::consts::TAU * ((k * 2) % 5) as f64 / 5.0 - std::f64::consts::FRAC_PI_2;
            (c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    path(&pts, true)
}

/// cmap + advances only (no shaping), same as phase 1's tile.
pub fn simple_glyphs(s: &str, size: f32) -> Vec<Glyph> {
    use skrifa::MetadataProvider;
    let Ok(font) = skrifa::FontRef::new(common::FONT_BYTES) else { return vec![] };
    let cmap = font.charmap();
    let metrics = font.glyph_metrics(skrifa::instance::Size::new(size), skrifa::instance::LocationRef::default());
    let mut pen = 0.0f32;
    s.chars()
        .map(|c| {
            let id = cmap.map(c).unwrap_or_default();
            let g = Glyph { id: id.to_u32(), x: pen, y: 0.0 };
            pen += metrics.advance_width(id).unwrap_or(size * 0.5);
            g
        })
        .collect()
}

pub fn font() -> FontData {
    FontData::new(Blob::new(Arc::new(common::FONT_BYTES)), 0)
}

fn text<S: Sparse>(ctx: &mut S, res: &mut S::Res, s: &str, x: f64, y: f64, size: f32, color: Color) {
    ctx.set_transform(Affine::translate((x, y)));
    ctx.set_paint(color.into());
    let glyphs = simple_glyphs(s, size);
    ctx.glyphs(res, &font(), size, &glyphs);
}

/// Which blend modes to draw (`skip` holds modes known to panic on this renderer).
pub fn coverage<S: Sparse>(ctx: &mut S, res: &mut S::Res, image: &ImageSource, skip: &[&str]) {
    let id = Affine::IDENTITY;
    ctx.set_transform(id);
    ctx.set_paint(Color::from_rgba8(0xf4, 0xf1, 0xea, 0xff).into());
    ctx.fill_rect(&Rect::new(0., 0., COVERAGE.0 as f64, COVERAGE.1 as f64));
    let labels = [
        "fill rule: nonzero | evenodd", "lineJoin: miter | round | bevel", "lineCap: butt | round | square", "miterLimit 10 | 1, setLineDash",
        "createLinearGradient", "createRadialGradient (2-point)", "createConicGradient (sweep)", "clip() to a path",
        "globalAlpha 0.5 | layer alpha", "shadowBlur ~ blurred rounded rect", "drawImage (scaled, rotated)", "fillText (glyph outlines)",
    ];
    for (i, l) in labels.iter().enumerate() {
        let (x, y) = tile(i);
        ctx.set_transform(id);
        ctx.set_paint(Color::from_rgba8(0, 0, 0, 0x30).into());
        ctx.set_stroke(Stroke::new(1.0));
        ctx.stroke_path(&Rect::new(x + 0.5, y + 0.5, x + TILE.0 - 0.5, y + TILE.1 - 0.5).to_path(TOLERANCE));
        text(ctx, res, l, x + 12.0, y + 26.0, 18.0, INK);
    }
    ctx.set_transform(id);
    // 0 fill rules
    let (x, y) = tile(0);
    ctx.set_paint(BLUE.into());
    ctx.set_fill_rule(Fill::NonZero);
    ctx.fill_path(&pentagram(Point::new(x + 110.0, y + 170.0), 90.0));
    ctx.set_fill_rule(Fill::EvenOdd);
    ctx.fill_path(&pentagram(Point::new(x + 290.0, y + 170.0), 90.0));
    ctx.set_fill_rule(Fill::NonZero);
    // 1 joins
    let (x, y) = tile(1);
    ctx.set_paint(TEAL.into());
    for (k, join) in [Join::Miter, Join::Round, Join::Bevel].into_iter().enumerate() {
        let ox = x + 30.0 + k as f64 * 120.0;
        ctx.set_stroke(Stroke::new(22.0).with_join(join));
        ctx.stroke_path(&path(&[(ox, y + 250.0), (ox + 45.0, y + 80.0), (ox + 90.0, y + 250.0)], false));
    }
    // 2 caps
    let (x, y) = tile(2);
    for (k, cap) in [gfx::kurbo::Cap::Butt, gfx::kurbo::Cap::Round, gfx::kurbo::Cap::Square].into_iter().enumerate() {
        let ly = y + 90.0 + k as f64 * 70.0;
        ctx.set_paint(RED.into());
        ctx.set_stroke(Stroke::new(30.0).with_caps(cap));
        ctx.stroke_path(&Line::new((x + 80.0, ly), (x + 320.0, ly)).to_path(TOLERANCE));
        ctx.set_paint(INK.into());
        ctx.set_stroke(Stroke::new(1.0));
        ctx.stroke_path(&Line::new((x + 80.0, ly - 30.0), (x + 80.0, ly + 30.0)).to_path(TOLERANCE));
        ctx.stroke_path(&Line::new((x + 320.0, ly - 30.0), (x + 320.0, ly + 30.0)).to_path(TOLERANCE));
    }
    // 3 miter limit + dashes
    let (x, y) = tile(3);
    ctx.set_paint(BLUE.into());
    for (k, limit) in [10.0, 1.0].into_iter().enumerate() {
        let ox = x + 40.0 + k as f64 * 110.0;
        ctx.set_stroke(Stroke::new(14.0).with_join(Join::Miter).with_miter_limit(limit));
        ctx.stroke_path(&path(&[(ox, y + 160.0), (ox + 40.0, y + 60.0), (ox + 80.0, y + 160.0)], false));
    }
    ctx.set_paint(RED.into());
    ctx.set_stroke(Stroke::new(8.0).with_caps(gfx::kurbo::Cap::Round).with_dashes(6.0, [24.0, 14.0, 4.0, 14.0]));
    ctx.stroke_path(&Circle::new((x + 320.0, y + 120.0), 55.0).to_path(TOLERANCE));
    ctx.set_paint(INK.into());
    ctx.set_stroke(Stroke::new(6.0).with_dashes(0.0, [20.0, 10.0]));
    ctx.stroke_path(&Line::new((x + 30.0, y + 250.0), (x + 370.0, y + 250.0)).to_path(TOLERANCE));
    // 4-6 gradients
    let (x, y) = tile(4);
    ctx.set_paint(Gradient::new_linear((x + 40.0, y + 60.0), (x + 360.0, y + 260.0)).with_stops([RED, GOLD, TEAL]).into());
    ctx.fill_path(&RoundedRect::new(x + 40.0, y + 60.0, x + 360.0, y + 270.0, 16.0).to_path(TOLERANCE));
    let (x, y) = tile(5);
    ctx.set_paint(Gradient::new_two_point_radial((x + 170.0, y + 140.0), 10.0, (x + 200.0, y + 170.0), 110.0).with_stops([GOLD, RED, BLUE]).into());
    ctx.fill_rect(&Rect::new(x + 40.0, y + 50.0, x + 360.0, y + 285.0));
    let (x, y) = tile(6);
    ctx.set_paint(Gradient::new_sweep((x + 200.0, y + 165.0), 0.0, std::f32::consts::TAU).with_stops([RED, GOLD, TEAL, BLUE, RED]).into());
    ctx.fill_path(&Circle::new((x + 200.0, y + 165.0), 110.0).to_path(TOLERANCE));
    // 7 clip
    let (x, y) = tile(7);
    ctx.push_layer(Some(&star(Point::new(x + 200.0, y + 170.0), 115.0, 50.0, 6)), None, None);
    for k in 0..14 {
        let sx = x + 60.0 + k as f64 * 22.0;
        ctx.set_paint(if k % 2 == 0 { BLUE } else { GOLD }.into());
        ctx.fill_rect(&Rect::new(sx, y + 40.0, sx + 22.0, y + 290.0));
    }
    ctx.pop_layer();
    // 8 alpha
    let (x, y) = tile(8);
    for (k, c) in [RED, TEAL, BLUE].into_iter().enumerate() {
        ctx.set_paint(c.multiply_alpha(0.5).into());
        ctx.fill_path(&Circle::new((x + 70.0 + k as f64 * 40.0, y + 160.0), 50.0).to_path(TOLERANCE));
    }
    ctx.push_layer(None, None, Some(0.5));
    for (k, c) in [RED, TEAL, BLUE].into_iter().enumerate() {
        ctx.set_paint(c.into());
        ctx.fill_path(&Circle::new((x + 250.0 + k as f64 * 40.0, y + 160.0), 50.0).to_path(TOLERANCE));
    }
    ctx.pop_layer();
    // 9 shadow
    let (x, y) = tile(9);
    ctx.set_paint(Color::from_rgba8(0, 0, 0, 0x90).into());
    ctx.fill_blurred_rounded_rect(&Rect::new(x + 90.0, y + 90.0, x + 330.0, y + 240.0), 18.0, 14.0);
    ctx.set_paint(Color::WHITE.into());
    ctx.fill_path(&RoundedRect::new(x + 80.0, y + 75.0, x + 320.0, y + 225.0, 18.0).to_path(TOLERANCE));
    // 10 image
    let (x, y) = tile(10);
    let xf = Affine::translate((x + 200.0, y + 165.0)) * Affine::rotate(0.3) * Affine::scale(3.0) * Affine::translate((-32.0, -32.0));
    ctx.set_transform(xf);
    ctx.set_paint(Brush::Image(ImageBrush { image: image.clone(), sampler: ImageSampler::default() }));
    ctx.fill_rect(&Rect::new(0., 0., 64., 64.));
    ctx.set_transform(id);
    // 11 text
    let (x, y) = tile(11);
    text(ctx, res, "Deka canvas", x + 30.0, y + 120.0, 48.0, BLUE);
    text(ctx, res, "Hello, vello sparse strips", x + 30.0, y + 180.0, 32.0, RED);
    text(ctx, res, "small 12px text for hinting", x + 30.0, y + 230.0, 12.0, INK);
    // 12-15 blend modes, each isolated on its own swatch
    let (gx, gy) = tile(12);
    let (sw, sh) = (1600.0 / 7.0, 300.0 / 4.0);
    for (k, (name, mix, compose)) in BLEND_MODES.iter().enumerate() {
        let (cx, cy) = (gx + (k % 7) as f64 * sw, gy + (k / 7) as f64 * sh);
        let cell = Rect::new(cx + 4.0, cy + 4.0, cx + 70.0, cy + sh - 4.0);
        ctx.set_transform(id);
        for j in 0..4 {
            for i in 0..4 {
                if (i + j) % 2 == 0 {
                    let (qx, qy) = (cell.x0 + i as f64 * 16.5, cell.y0 + j as f64 * 16.75);
                    ctx.set_paint(Color::from_rgba8(0xcc, 0xcc, 0xcc, 0xff).into());
                    ctx.fill_rect(&Rect::new(qx, qy, qx + 16.5, qy + 16.75));
                }
            }
        }
        if skip.contains(name) {
            ctx.set_paint(RED.into());
            ctx.set_stroke(Stroke::new(3.0));
            ctx.stroke_path(&path(&[(cell.x0, cell.y0), (cell.x1, cell.y1)], false));
            ctx.stroke_path(&path(&[(cell.x1, cell.y0), (cell.x0, cell.y1)], false));
        } else {
            ctx.push_layer(Some(&cell.to_path(TOLERANCE)), None, None);
            ctx.set_paint(BLUE.into());
            ctx.fill_rect(&Rect::new(cell.x0 + 6.0, cell.y0 + 6.0, cell.x0 + 42.0, cell.y0 + 42.0));
            ctx.push_layer(Some(&cell.to_path(TOLERANCE)), Some(BlendMode::new(*mix, *compose)), None);
            ctx.set_paint(Color::from_rgba8(0xe6, 0x39, 0x46, 0xd0).into());
            ctx.fill_path(&Circle::new((cell.x0 + 42.0, cell.y0 + 42.0), 20.0).to_path(TOLERANCE));
            ctx.pop_layer();
            ctx.pop_layer();
        }
        text(ctx, res, name, cx + 76.0, cy + sh / 2.0 + 5.0, 14.0, INK);
    }
}

/// 64x64 checker/gradient image used by the coverage sheet.
pub fn coverage_image() -> GlyphImage {
    let (iw, ih) = (64usize, 64usize);
    let mut px = Vec::with_capacity(iw * ih * 4);
    for j in 0..ih {
        for i in 0..iw {
            let check = ((i / 8) + (j / 8)) % 2 == 0;
            let (r, g, b) = if check { (230, 57, 70) } else { (i as u8 * 4, j as u8 * 4, 200) };
            px.extend_from_slice(&[r, g, b, 255]);
        }
    }
    GlyphImage { id: "coverage".into(), width: iw, height: ih, rgba: px }
}

// ---------------------------------------------------------------------------
// Generic winit app over a presenter

#[derive(Default, Clone, Copy)]
pub struct StartupTimes {
    pub window_ms: f64,
    pub device_ms: f64,
    pub renderer_ms: f64,
}

pub enum Presented {
    Done,
    /// Window not visible: draw nothing, wait for Occluded(false).
    Occluded,
    Retry,
}

pub enum Content {
    World(World),
    Ui { host: Host<common::UiApp>, renderer: deka_native_ui::scene::Renderer, scene: DekaScene },
    Canvas { graph: graph::Graph, run: usize },
}

impl Content {
    pub fn graph(&self) -> Option<&graph::Graph> {
        match self {
            Content::Canvas { graph, .. } => Some(graph),
            _ => None,
        }
    }
}

/// What differs between vello_gpu and vello_cpu.
pub trait Presenter: Sized {
    const NAME: &'static str;
    fn new(window: Arc<Window>, times: &mut StartupTimes) -> Result<Self, String>;
    fn resize(&mut self, size: PhysicalSize<u32>);
    /// Encode a deka scene (uploading its images as needed).
    fn encode_deka(&mut self, deka: &DekaScene, scale: f64);
    fn encode_graph(&mut self, graph: &graph::Graph, scale: f64);
    /// Render and present; returns how it went. Time spent waiting for the
    /// drawable/display is reported separately via `present_wait`.
    fn present(&mut self) -> Result<Presented, String>;
    fn screenshot(&mut self) -> Option<(u32, u32, Vec<u8>)>;
}

struct SparseApp<P: Presenter> {
    args: Args,
    protocol: Protocol,
    times: StartupTimes,
    window: Option<Arc<Window>>,
    presenter: Option<P>,
    content: Option<Content>,
    clock: Instant,
    idle_deadline: Option<Instant>,
    quit: bool,
    occluded: bool,
    cursor: (f64, f64),
}

impl<P: Presenter> SparseApp<P> {
    fn scale(&self) -> f64 {
        self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(2.)
    }

    fn teardown(&mut self, event_loop: &ActiveEventLoop) {
        let t = Instant::now();
        if !self.args.first_frame && !self.args.resize && let (Some(p), Some(c)) = (self.presenter.as_mut(), &self.content) {
            let name = match c {
                Content::World(_) => "world",
                Content::Ui { .. } => "ui",
                Content::Canvas { .. } => "canvas-10k",
            };
            if let Some((w, h, rgba)) = p.screenshot() {
                common::write_png(&common::shots_dir().join(format!("{name}-{}.png", P::NAME)), w, h, &rgba);
            }
        }
        println!("[{}] GPU memory before teardown: {:.1} MB", P::NAME, common::gpu_mb());
        self.content = None;
        self.presenter = None;
        println!("[{}] teardown {:.2} ms", P::NAME, t.elapsed().as_secs_f64() * 1000.);
        event_loop.exit();
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            self.teardown(event_loop);
            return;
        }
        if self.occluded {
            return;
        }
        if let Step::Quit = self.protocol.begin_frame() {
            self.teardown(event_loop);
            return;
        }
        let t0 = Instant::now();
        if self.args.resize
            && let Some(w) = &self.window
        {
            let (lw, lh) = common::resize_size(self.protocol.frame_index());
            let _ = w.request_inner_size(LogicalSize::new(lw, lh));
        }
        let scale = self.scale();
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (lw, lh) = (size.width as f64 / scale, size.height as f64 / scale);
        let ms = self.clock.elapsed().as_secs_f64() * 1000.;
        let (Some(presenter), Some(content)) = (self.presenter.as_mut(), self.content.as_mut()) else { return };
        let mut animating = true;
        match content {
            Content::World(world) => {
                let s = world.frame(lw as f32, lh as f32, ms, false);
                animating = s.animating;
                presenter.encode_deka(&s, scale);
            }
            Content::Ui { host, renderer, scene } => {
                *scene = renderer.render_at(&host.render(), lw as f32, lh as f32, scale as f32, ms, false);
                animating = scene.animating;
                presenter.encode_deka(scene, scale);
            }
            Content::Canvas { graph, .. } => {
                graph.update(ms / 1000.);
                presenter.encode_graph(graph, scale);
            }
        }
        let work_encode = t0.elapsed();
        let tp = Instant::now();
        match presenter.present() {
            Ok(Presented::Done) => {}
            Ok(Presented::Occluded) => {
                if !self.occluded {
                    eprintln!("[{}] occluded: pausing until visible", P::NAME);
                }
                self.occluded = true;
                return;
            }
            Ok(Presented::Retry) => {
                window.request_redraw();
                return;
            }
            Err(e) => {
                eprintln!("[{}] {e}", P::NAME);
                self.quit = true;
                window.request_redraw();
                return;
            }
        }
        // For sparse renderers the CPU raster/strip work happens inside
        // present(); report it as part of our work, and the whole call too.
        let present = tp.elapsed();
        if self.protocol.measuring() {
            self.protocol.present.push(present);
        }
        let first = !self.protocol.first_frame_done();
        let step = self.protocol.end_frame(work_encode + present);
        if first {
            println!(
                "[{}] startup: window {:.1} ms, device {:.1} ms, renderer setup (pipelines/threads) {:.1} ms, first frame work {:.1} ms; GPU {:.1} MB",
                P::NAME, self.times.window_ms, self.times.device_ms, self.times.renderer_ms,
                (work_encode + present).as_secs_f64() * 1000., common::gpu_mb()
            );
            if let Some(secs) = self.args.idle_secs {
                self.idle_deadline = Some(Instant::now() + Duration::from_secs_f64(secs));
            }
        }
        if let Step::Quit = step {
            if let Some(Content::Canvas { run, .. }) = self.content.as_ref()
                && run + 1 < self.args.shapes.len()
            {
                let run = run + 1;
                let shapes = self.args.shapes[run];
                let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
                self.content = Some(Content::Canvas { graph: graph::Graph::new(shapes, lw, lh, layout), run });
                self.protocol = Protocol::new(self.args.clone(), P::NAME);
                println!("[{}] canvas shapes {shapes} ({layout:?})", P::NAME);
            } else {
                self.quit = true;
            }
        }
        if self.quit || self.args.first_frame || (self.args.idle_secs.is_none()) || (animating && self.args.idle_secs.is_none()) {
            window.request_redraw();
        }
    }
}

impl<P: Presenter> ApplicationHandler for SparseApp<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        common::trace("resumed");
        let t = Instant::now();
        let mut attrs = Window::default_attributes()
            .with_title(format!("deka A/B: winit + {}", P::NAME))
            .with_inner_size(LogicalSize::new(WINDOW_W as f64, WINDOW_H as f64));
        if !self.args.interactive {
            attrs = attrs.with_active(false).with_window_level(WindowLevel::AlwaysOnTop);
            if let Some(m) = event_loop.primary_monitor() {
                let s = m.size().to_logical::<f64>(m.scale_factor());
                attrs = attrs.with_position(LogicalPosition::new(s.width - WINDOW_W as f64 - 8., s.height - WINDOW_H as f64 - 8.));
            }
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("create window: {e}");
                event_loop.exit();
                return;
            }
        };
        self.times.window_ms = t.elapsed().as_secs_f64() * 1000.;
        let presenter = match P::new(window.clone(), &mut self.times) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[{}] {e}", P::NAME);
                event_loop.exit();
                return;
            }
        };
        common::trace("presenter ready");
        let scale = window.scale_factor();
        let size = window.inner_size();
        let (lw, lh) = (size.width as f64 / scale, size.height as f64 / scale);
        self.content = Some(match self.args.app {
            AppKind::Ui => Content::Ui { host: Host::new(common::UiApp), renderer: deka_native_ui::scene::Renderer::new(), scene: DekaScene::default() },
            AppKind::Canvas => {
                let shapes = self.args.shapes.first().copied().unwrap_or(1_000);
                let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
                println!("[{}] canvas shapes {shapes} ({layout:?})", P::NAME);
                Content::Canvas { graph: graph::Graph::new(shapes, lw, lh, layout), run: 0 }
            }
            _ => {
                let mut world = World::new();
                world.start();
                world.set_muted(true);
                Content::World(world)
            }
        });
        self.presenter = Some(presenter);
        self.window = Some(window.clone());
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.teardown(event_loop),
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded && let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(p) = self.presenter.as_mut() {
                    p.resize(size);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if !self.protocol.first_frame_done() {
                    common::trace("RedrawRequested");
                }
                self.frame(event_loop);
            }
            WindowEvent::CursorMoved { position, .. } => self.cursor = (position.x, position.y),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let scale = self.scale();
                if let Some(Content::Ui { host, scene, .. }) = self.content.as_mut()
                    && let Some(handler) = scene.hit((self.cursor.0 / scale) as f32, (self.cursor.1 / scale) as f32).map(|t| t.handler)
                {
                    host.click(handler);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(Content::World(world)) = self.content.as_mut() {
                    let name = match &event.logical_key {
                        Key::Named(NamedKey::ArrowUp) => "up".to_owned(),
                        Key::Named(NamedKey::ArrowDown) => "down".to_owned(),
                        Key::Named(NamedKey::ArrowLeft) => "left".to_owned(),
                        Key::Named(NamedKey::ArrowRight) => "right".to_owned(),
                        Key::Named(NamedKey::Enter) => "enter".to_owned(),
                        Key::Named(NamedKey::Space) => "space".to_owned(),
                        Key::Character(c) => c.to_string(),
                        _ => String::new(),
                    };
                    world.key(&name, event.state == ElementState::Pressed);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(deadline) = self.idle_deadline {
            if Instant::now() >= deadline {
                self.protocol.idle_done();
                self.idle_deadline = None;
                self.teardown(event_loop);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

pub fn run_windowed<P: Presenter>(args: Args) {
    common::trace("main");
    let event_loop = match EventLoop::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("event loop: {e}");
            return;
        }
    };
    let mut app = SparseApp::<P> {
        protocol: Protocol::new(args.clone(), P::NAME),
        args,
        times: StartupTimes::default(),
        window: None,
        presenter: None,
        content: None,
        clock: Instant::now(),
        idle_deadline: None,
        quit: false,
        occluded: false,
        cursor: (0., 0.),
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop: {e}");
    }
}

/// Image bookkeeping shared by presenters: which deka image ids a scene uses
/// and which cached ones can be dropped.
pub fn used_ids(deka: &DekaScene) -> HashSet<&str> {
    deka.paint.iter().filter_map(|p| p.image.as_deref()).collect()
}

/// Filter effects (what Canvas `shadowBlur` on any shape and `filter: blur()`
/// need, and compute vello lacks): a drop shadow on a star, blurred text.
pub const FILTER_SHEET: (u32, u32) = (800, 300);
pub fn filter_sheet<S: Sparse>(ctx: &mut S, res: &mut S::Res) {
    use filter_effects::{EdgeMode, Filter, FilterPrimitive};
    ctx.set_transform(Affine::IDENTITY);
    ctx.set_paint(Color::from_rgba8(0xf4, 0xf1, 0xea, 0xff).into());
    ctx.fill_rect(&Rect::new(0., 0., FILTER_SHEET.0 as f64, FILTER_SHEET.1 as f64));
    text(ctx, res, "DropShadow filter on a path", 20., 30., 18., INK);
    text(ctx, res, "GaussianBlur filter on text", 420., 30., 18., INK);
    ctx.set_transform(Affine::IDENTITY);
    ctx.push_filter_layer(Filter::from_primitive(FilterPrimitive::DropShadow {
        dx: 8.,
        dy: 10.,
        std_deviation: 6.,
        color: Color::from_rgba8(0, 0, 0, 0xa0),
        edge_mode: EdgeMode::None,
    }));
    ctx.set_paint(GOLD.into());
    ctx.fill_path(&star(Point::new(190., 170.), 100., 45., 5));
    ctx.pop_layer();
    ctx.set_transform(Affine::IDENTITY);
    ctx.push_filter_layer(Filter::from_primitive(FilterPrimitive::GaussianBlur { std_deviation: 3., edge_mode: EdgeMode::None }));
    text(ctx, res, "blurred", 450., 190., 64., BLUE);
    ctx.pop_layer();
}
