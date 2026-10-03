//! A sheet that exercises the Canvas 2D features we care about, rendered by
//! vello into `shots/coverage.png`. Each tile is labelled with vello-drawn text.

use std::sync::Arc;

use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use vello::kurbo::{Affine, BezPath, Cap, Circle, Join, Line, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{
    Blob, BlendMode, Color, Compose, Fill, FontData, Gradient, ImageAlphaType, ImageBrush,
    ImageData, ImageFormat, Mix,
};
use vello::{Glyph, Scene};

pub const SIZE: (u32, u32) = (1600, 1200);
pub const BACKGROUND: Color = Color::from_rgba8(0xf4, 0xf1, 0xea, 0xff);
const TILE: (f64, f64) = (400.0, 300.0);
const INK: Color = Color::from_rgba8(0x22, 0x22, 0x22, 0xff);
const RED: Color = Color::from_rgba8(0xe6, 0x39, 0x46, 0xff);
const BLUE: Color = Color::from_rgba8(0x1d, 0x35, 0x57, 0xff);
const TEAL: Color = Color::from_rgba8(0x2a, 0x9d, 0x8f, 0xff);
const GOLD: Color = Color::from_rgba8(0xe9, 0xc4, 0x6a, 0xff);

static FONT_BYTES: &[u8] =
    include_bytes!("../../../crates/deka_native_ui/assets/AtkinsonHyperlegible-Regular.ttf");

struct Text {
    font: FontData,
}

impl Text {
    fn new() -> Self {
        Text { font: FontData::new(Blob::new(Arc::new(FONT_BYTES)), 0) }
    }
    /// No shaping: cmap lookup + advances only (what a canvas `fillText`
    /// needs is a real shaper; see the spike PR).
    fn draw(&self, scene: &mut Scene, s: &str, x: f64, y: f64, size: f32, color: Color) {
        let fr = skrifa::FontRef::new(FONT_BYTES).expect("font");
        let cmap = fr.charmap();
        let metrics = fr.glyph_metrics(Size::new(size), LocationRef::default());
        let mut pen = 0.0f32;
        let glyphs = s
            .chars()
            .map(|c| {
                let id = cmap.map(c).unwrap_or_default();
                let g = Glyph { id: id.to_u32(), x: pen, y: 0.0 };
                pen += metrics.advance_width(id).unwrap_or(size * 0.5);
                g
            })
            .collect::<Vec<_>>();
        scene
            .draw_glyphs(&self.font)
            .font_size(size)
            .transform(Affine::translate((x, y)))
            .brush(color)
            .draw(Fill::NonZero, glyphs.into_iter());
    }
}

fn star(c: Point, r_out: f64, r_in: f64, points: usize) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..points * 2 {
        let r = if i % 2 == 0 { r_out } else { r_in };
        let a = std::f64::consts::PI * i as f64 / points as f64 - std::f64::consts::FRAC_PI_2;
        let pt = Point::new(c.x + r * a.cos(), c.y + r * a.sin());
        if i == 0 { p.move_to(pt) } else { p.line_to(pt) }
    }
    p.close_path();
    p
}

/// Pentagram drawn as one self-intersecting path: nonzero fills the centre,
/// evenodd leaves it empty.
fn pentagram(c: Point, r: f64) -> BezPath {
    let mut p = BezPath::new();
    for k in 0..5 {
        let i = (k * 2) % 5;
        let a = std::f64::consts::TAU * i as f64 / 5.0 - std::f64::consts::FRAC_PI_2;
        let pt = Point::new(c.x + r * a.cos(), c.y + r * a.sin());
        if k == 0 { p.move_to(pt) } else { p.line_to(pt) }
    }
    p.close_path();
    p
}

fn tile(i: usize) -> (f64, f64) {
    ((i % 4) as f64 * TILE.0, (i / 4) as f64 * TILE.1)
}

pub fn build(scene: &mut Scene) {
    let text = Text::new();
    let id = Affine::IDENTITY;
    let label = |scene: &mut Scene, i: usize, s: &str| {
        let (x, y) = tile(i);
        scene.stroke(
            &Stroke::new(1.0),
            id,
            Color::from_rgba8(0, 0, 0, 0x30),
            None,
            &Rect::new(x + 0.5, y + 0.5, x + TILE.0 - 0.5, y + TILE.1 - 0.5),
        );
        text.draw(scene, s, x + 12.0, y + 26.0, 18.0, INK);
    };

    // 0 fill rules
    let (x, y) = tile(0);
    label(scene, 0, "fill rule: nonzero | evenodd");
    scene.fill(Fill::NonZero, id, BLUE, None, &pentagram(Point::new(x + 110.0, y + 170.0), 90.0));
    scene.fill(Fill::EvenOdd, id, BLUE, None, &pentagram(Point::new(x + 290.0, y + 170.0), 90.0));

    // 1 joins
    let (x, y) = tile(1);
    label(scene, 1, "lineJoin: miter | round | bevel");
    for (k, join) in [Join::Miter, Join::Round, Join::Bevel].into_iter().enumerate() {
        let ox = x + 30.0 + k as f64 * 120.0;
        let mut p = BezPath::new();
        p.move_to((ox, y + 250.0));
        p.line_to((ox + 45.0, y + 80.0));
        p.line_to((ox + 90.0, y + 250.0));
        scene.stroke(&Stroke::new(22.0).with_join(join), id, TEAL, None, &p);
    }

    // 2 caps
    let (x, y) = tile(2);
    label(scene, 2, "lineCap: butt | round | square");
    for (k, cap) in [Cap::Butt, Cap::Round, Cap::Square].into_iter().enumerate() {
        let ly = y + 90.0 + k as f64 * 70.0;
        scene.stroke(&Stroke::new(30.0).with_caps(cap), id, RED, None, &Line::new((x + 80.0, ly), (x + 320.0, ly)));
        scene.stroke(&Stroke::new(1.0), id, INK, None, &Line::new((x + 80.0, ly - 30.0), (x + 80.0, ly + 30.0)));
        scene.stroke(&Stroke::new(1.0), id, INK, None, &Line::new((x + 320.0, ly - 30.0), (x + 320.0, ly + 30.0)));
    }

    // 3 miter limit + dashes
    let (x, y) = tile(3);
    label(scene, 3, "miterLimit 10 | 1, setLineDash");
    for (k, limit) in [10.0, 1.0].into_iter().enumerate() {
        let ox = x + 40.0 + k as f64 * 110.0;
        let mut p = BezPath::new();
        p.move_to((ox, y + 160.0));
        p.line_to((ox + 40.0, y + 60.0));
        p.line_to((ox + 80.0, y + 160.0));
        scene.stroke(&Stroke::new(14.0).with_join(Join::Miter).with_miter_limit(limit), id, BLUE, None, &p);
    }
    scene.stroke(
        &Stroke::new(8.0).with_caps(Cap::Round).with_dashes(6.0, [24.0, 14.0, 4.0, 14.0]),
        id,
        RED,
        None,
        &Circle::new((x + 320.0, y + 120.0), 55.0),
    );
    scene.stroke(
        &Stroke::new(6.0).with_dashes(0.0, [20.0, 10.0]),
        id,
        INK,
        None,
        &Line::new((x + 30.0, y + 250.0), (x + 370.0, y + 250.0)),
    );

    // 4 linear gradient
    let (x, y) = tile(4);
    label(scene, 4, "createLinearGradient");
    let g = Gradient::new_linear((x + 40.0, y + 60.0), (x + 360.0, y + 260.0)).with_stops([RED, GOLD, TEAL]);
    scene.fill(Fill::NonZero, id, &g, None, &RoundedRect::new(x + 40.0, y + 60.0, x + 360.0, y + 270.0, 16.0));

    // 5 radial gradient (two-point conical, as canvas)
    let (x, y) = tile(5);
    label(scene, 5, "createRadialGradient (2-point)");
    let g = Gradient::new_two_point_radial((x + 170.0, y + 140.0), 10.0, (x + 200.0, y + 170.0), 110.0)
        .with_stops([GOLD, RED, BLUE]);
    scene.fill(Fill::NonZero, id, &g, None, &Rect::new(x + 40.0, y + 50.0, x + 360.0, y + 285.0));

    // 6 conic / sweep
    let (x, y) = tile(6);
    label(scene, 6, "createConicGradient (sweep)");
    let g = Gradient::new_sweep((x + 200.0, y + 165.0), 0.0, std::f32::consts::TAU)
        .with_stops([RED, GOLD, TEAL, BLUE, RED]);
    scene.fill(Fill::NonZero, id, &g, None, &Circle::new((x + 200.0, y + 165.0), 110.0));

    // 7 clip to arbitrary path
    let (x, y) = tile(7);
    label(scene, 7, "clip() to a path");
    scene.push_clip_layer(Fill::NonZero, id, &star(Point::new(x + 200.0, y + 170.0), 115.0, 50.0, 6));
    for k in 0..14 {
        let sx = x + 60.0 + k as f64 * 22.0;
        scene.fill(Fill::NonZero, id, if k % 2 == 0 { BLUE } else { GOLD }, None, &Rect::new(sx, y + 40.0, sx + 22.0, y + 290.0));
    }
    scene.pop_layer();

    // 8 globalAlpha (group) vs per-shape alpha
    let (x, y) = tile(8);
    label(scene, 8, "globalAlpha 0.5 | layer alpha");
    for k in 0..3 {
        let c = [RED, TEAL, BLUE][k];
        scene.fill(Fill::NonZero, id, c.multiply_alpha(0.5), None, &Circle::new((x + 70.0 + k as f64 * 40.0, y + 160.0), 50.0));
    }
    scene.push_layer(Fill::NonZero, Mix::Normal, 0.5, id, &Rect::new(x + 200.0, y + 40.0, x + 400.0, y + 300.0));
    for k in 0..3 {
        let c = [RED, TEAL, BLUE][k];
        scene.fill(Fill::NonZero, id, c, None, &Circle::new((x + 250.0 + k as f64 * 40.0, y + 160.0), 50.0));
    }
    scene.pop_layer();

    // 9 shadow: blurred rounded rect (vello's only blur primitive)
    let (x, y) = tile(9);
    label(scene, 9, "shadowBlur ~ blurred rounded rect");
    scene.draw_blurred_rounded_rect(id, Rect::new(x + 90.0, y + 90.0, x + 330.0, y + 240.0), Color::from_rgba8(0, 0, 0, 0x90), 18.0, 14.0);
    scene.fill(Fill::NonZero, id, Color::WHITE, None, &RoundedRect::new(x + 80.0, y + 75.0, x + 320.0, y + 225.0, 18.0));

    // 10 drawImage (scaled + rotated)
    let (x, y) = tile(10);
    label(scene, 10, "drawImage (scaled, rotated)");
    let (iw, ih) = (64u32, 64u32);
    let mut px = Vec::with_capacity((iw * ih * 4) as usize);
    for j in 0..ih {
        for i in 0..iw {
            let check = ((i / 8) + (j / 8)) % 2 == 0;
            let (r, g, b) = if check { (230, 57, 70) } else { (i as u8 * 4, j as u8 * 4, 200) };
            px.extend_from_slice(&[r, g, b, 255]);
        }
    }
    let img = ImageData { data: Blob::new(Arc::new(px)), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: iw, height: ih };
    scene.draw_image(&ImageBrush::new(img), Affine::translate((x + 200.0, y + 165.0)) * Affine::rotate(0.3) * Affine::scale(3.0) * Affine::translate((-32.0, -32.0)));

    // 11 text
    let (x, y) = tile(11);
    label(scene, 11, "fillText (glyph outlines)");
    text.draw(scene, "Deka canvas", x + 30.0, y + 120.0, 48.0, BLUE);
    text.draw(scene, "Hello, vello 0.11", x + 30.0, y + 180.0, 32.0, RED);
    text.draw(scene, "small 12px text for hinting", x + 30.0, y + 230.0, 12.0, INK);

    // 12-15 globalCompositeOperation: every canvas mode on its own isolated swatch.
    let modes: [(&str, BlendMode); 26] = [
        ("source-over", BlendMode::new(Mix::Normal, Compose::SrcOver)),
        ("source-in", BlendMode::new(Mix::Normal, Compose::SrcIn)),
        ("source-out", BlendMode::new(Mix::Normal, Compose::SrcOut)),
        ("source-atop", BlendMode::new(Mix::Normal, Compose::SrcAtop)),
        ("destination-over", BlendMode::new(Mix::Normal, Compose::DestOver)),
        ("destination-in", BlendMode::new(Mix::Normal, Compose::DestIn)),
        ("destination-out", BlendMode::new(Mix::Normal, Compose::DestOut)),
        ("destination-atop", BlendMode::new(Mix::Normal, Compose::DestAtop)),
        ("lighter", BlendMode::new(Mix::Normal, Compose::Plus)),
        ("copy", BlendMode::new(Mix::Normal, Compose::Copy)),
        ("xor", BlendMode::new(Mix::Normal, Compose::Xor)),
        ("multiply", BlendMode::new(Mix::Multiply, Compose::SrcOver)),
        ("screen", BlendMode::new(Mix::Screen, Compose::SrcOver)),
        ("overlay", BlendMode::new(Mix::Overlay, Compose::SrcOver)),
        ("darken", BlendMode::new(Mix::Darken, Compose::SrcOver)),
        ("lighten", BlendMode::new(Mix::Lighten, Compose::SrcOver)),
        ("color-dodge", BlendMode::new(Mix::ColorDodge, Compose::SrcOver)),
        ("color-burn", BlendMode::new(Mix::ColorBurn, Compose::SrcOver)),
        ("hard-light", BlendMode::new(Mix::HardLight, Compose::SrcOver)),
        ("soft-light", BlendMode::new(Mix::SoftLight, Compose::SrcOver)),
        ("difference", BlendMode::new(Mix::Difference, Compose::SrcOver)),
        ("exclusion", BlendMode::new(Mix::Exclusion, Compose::SrcOver)),
        ("hue", BlendMode::new(Mix::Hue, Compose::SrcOver)),
        ("saturation", BlendMode::new(Mix::Saturation, Compose::SrcOver)),
        ("color", BlendMode::new(Mix::Color, Compose::SrcOver)),
        ("luminosity", BlendMode::new(Mix::Luminosity, Compose::SrcOver)),
    ];
    let (gx, gy) = tile(12);
    let (sw, sh) = (1600.0 / 7.0, 300.0 / 4.0);
    for (k, (name, mode)) in modes.iter().enumerate() {
        let (cx, cy) = (gx + (k % 7) as f64 * sw, gy + (k / 7) as f64 * sh);
        let cell = Rect::new(cx + 4.0, cy + 4.0, cx + 70.0, cy + sh - 4.0);
        // Checkerboard so transparency is visible.
        for j in 0..4 {
            for i in 0..4 {
                if (i + j) % 2 == 0 {
                    let (qx, qy) = (cell.x0 + i as f64 * 16.5, cell.y0 + j as f64 * 16.75);
                    scene.fill(Fill::NonZero, id, Color::from_rgba8(0xcc, 0xcc, 0xcc, 0xff), None, &Rect::new(qx, qy, qx + 16.5, qy + 16.75));
                }
            }
        }
        // Isolated group = a fresh transparent canvas the size of the swatch.
        scene.push_layer(Fill::NonZero, Mix::Normal, 1.0, id, &cell);
        // destination: blue square
        scene.fill(Fill::NonZero, id, BLUE, None, &Rect::new(cell.x0 + 6.0, cell.y0 + 6.0, cell.x0 + 42.0, cell.y0 + 42.0));
        // source: red-ish circle with the mode
        scene.push_layer(Fill::NonZero, *mode, 1.0, id, &cell);
        scene.fill(Fill::NonZero, id, Color::from_rgba8(0xe6, 0x39, 0x46, 0xd0), None, &Circle::new((cell.x0 + 42.0, cell.y0 + 42.0), 20.0));
        scene.pop_layer();
        scene.pop_layer();
        text.draw(scene, name, cx + 76.0, cy + sh / 2.0 + 5.0, 14.0, INK);
    }
}
