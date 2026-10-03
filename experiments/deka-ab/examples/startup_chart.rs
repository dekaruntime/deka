//! Draw the phase-6 start-up waterfall as a PNG with vello_cpu. Input CSV
//! rows: `seg,<stack>,<category 1-4>,<start_ms>,<end_ms>` (main-thread
//! phases) and `mark,<stack>,<window on screen|frame on screen>,<ms>,<ms>`.
//! One horizontal bar per stack on a shared ms axis; categories use the
//! validated categorical slots in fixed order; markers for when the window
//! and the first frame reach the screen.
//!
//!   cargo run --release --features cpu-backend --example startup_chart -- data.csv out.png

use deka_ab::sparse::gfx::kurbo::{Affine, Circle, Line, Rect, RoundedRect, Shape, Stroke};
use deka_ab::sparse::gfx::peniko::Color;
use deka_ab::sparse::{Sparse, font, simple_glyphs};

const SURFACE: Color = Color::from_rgba8(0xfc, 0xfc, 0xfb, 0xff);
const INK: Color = Color::from_rgba8(0x0b, 0x0b, 0x0b, 0xff);
const INK2: Color = Color::from_rgba8(0x52, 0x51, 0x4e, 0xff);
const MUTED: Color = Color::from_rgba8(0x89, 0x87, 0x81, 0xff);
const GRID: Color = Color::from_rgba8(0xe1, 0xe0, 0xd9, 0xff);
const BASE: Color = Color::from_rgba8(0xc3, 0xc2, 0xb7, 0xff);
// Validated categorical slots 1-4 (dataviz reference palette), fixed order.
const SERIES: [Color; 4] = [
    Color::from_rgba8(0x2a, 0x78, 0xd6, 0xff),
    Color::from_rgba8(0xeb, 0x68, 0x34, 0xff),
    Color::from_rgba8(0x1b, 0xaf, 0x7a, 0xff),
    Color::from_rgba8(0xed, 0xa1, 0x00, 0xff),
];
const LABELS: [&str; 4] = [
    "process start to app launched (dyld, AppKit, event loop)",
    "window creation (GPUI: + Metal renderer)",
    "GPU device, renderer, fonts on the main thread",
    "frame 1: scene, strips, render (before: + visibility wait)",
];

#[allow(clippy::too_many_arguments, reason = "one call site per label; a struct would only rename the arguments")]
fn text(ctx: &mut vello_cpu::RenderContext, res: &mut vello_cpu::Resources, s: &str, x: f64, y: f64, size: f32, color: Color, right: bool) {
    let glyphs = simple_glyphs(s, size);
    let width = glyphs.last().map(|g| g.x as f64 + size as f64 * 0.55).unwrap_or(0.);
    ctx.set_transform(Affine::translate((if right { x - width } else { x }, y)));
    ctx.set_paint(color);
    Sparse::glyphs(ctx, res, &font(), size, &glyphs);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(input), Some(output)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: startup_chart data.csv out.png");
        return;
    };
    let Ok(csv) = std::fs::read_to_string(input) else {
        eprintln!("cannot read {input}");
        return;
    };
    // (kind, stack, category or marker, start, end); the stack may contain commas.
    let rows: Vec<(String, String, String, f64, f64)> = csv
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (kind, rest) = l.split_once(',')?;
            let f: Vec<&str> = rest.rsplitn(4, ',').map(str::trim).collect();
            Some((kind.to_owned(), f.get(3)?.to_string(), f.get(2)?.to_string(), f.get(1)?.parse().ok()?, f.first()?.parse().ok()?))
        })
        .collect();
    let mut stacks: Vec<String> = vec![];
    for r in &rows {
        if !stacks.contains(&r.1) {
            stacks.push(r.1.clone());
        }
    }
    // Headroom on the right for the last marker's label.
    let x_max = ((rows.iter().map(|r| r.4).fold(0., f64::max) + 90.) / 50.).ceil() * 50.;
    let (w, h) = (1120u16, (150. + stacks.len() as f64 * 92. + 70.) as u16);
    let (x0, x1) = (300., w as f64 - 40.);
    let xp = |ms: f64| x0 + ms / x_max * (x1 - x0);
    let mut ctx = vello_cpu::RenderContext::new(w, h);
    let mut res = vello_cpu::Resources::new();
    ctx.set_paint(SURFACE);
    ctx.fill_rect(&Rect::new(0., 0., w as f64, h as f64));
    text(&mut ctx, &mut res, "Start-up on the Intel iMac: main thread from process start to first frame (median of 7 warm launches)", 24., 32., 16., INK, false);
    let (top, bottom) = (70., 70. + stacks.len() as f64 * 92.);
    // x grid
    let mut v = 0.;
    while v <= x_max {
        ctx.set_transform(Affine::IDENTITY);
        ctx.set_paint(if v == 0. { BASE } else { GRID });
        ctx.set_stroke(Stroke::new(1.));
        ctx.stroke_path(&Line::new((xp(v), top), (xp(v), bottom)).to_path(0.1));
        text(&mut ctx, &mut res, &format!("{v:.0}"), xp(v) - 8., bottom + 18., 11., MUTED, false);
        v += 50.;
    }
    text(&mut ctx, &mut res, "ms since the process started", (x0 + x1) / 2. - 80., bottom + 38., 12., INK2, false);
    for (si, stack) in stacks.iter().enumerate() {
        let y = top + 20. + si as f64 * 92.;
        text(&mut ctx, &mut res, stack, x0 - 12., y + 20., 13., INK, true);
        let mut frame_done: f64 = 0.;
        for r in rows.iter().filter(|r| &r.1 == stack && r.0 == "seg") {
            let Ok(c) = r.2.parse::<usize>() else { continue };
            let Some(color) = SERIES.get(c.wrapping_sub(1)) else { continue };
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(*color);
            // 2px surface gap between adjacent segments.
            let (a, b) = (xp(r.3) + 1., (xp(r.4) - 1.).max(xp(r.3) + 2.));
            ctx.fill_path(&RoundedRect::new(a, y, b, y + 28., 3.).to_path(0.1));
            frame_done = frame_done.max(r.4);
        }
        // From frame 1 to the first frame on screen: the window server.
        let marks: Vec<&(String, String, String, f64, f64)> = rows.iter().filter(|r| &r.1 == stack && r.0 == "mark").collect();
        if let Some(shown) = marks.iter().find(|r| r.2 == "frame on screen") {
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(BASE);
            ctx.set_stroke(Stroke::new(2.).with_dashes(0., [3., 3.]));
            ctx.stroke_path(&Line::new((xp(frame_done), y + 14.), (xp(shown.3), y + 14.)).to_path(0.1));
        }
        for m in &marks {
            let x = xp(m.3);
            ctx.set_transform(Affine::IDENTITY);
            if m.2 == "window on screen" {
                // Diamond above the bar.
                let mut d = deka_ab::sparse::gfx::kurbo::BezPath::new();
                d.move_to((x, y - 12.));
                d.line_to((x + 6., y - 6.));
                d.line_to((x, y));
                d.line_to((x - 6., y - 6.));
                d.close_path();
                ctx.set_paint(INK2);
                ctx.fill_path(&d);
                text(&mut ctx, &mut res, &format!("window {:.0}", m.3), x + 8., y - 4., 11., INK2, false);
            } else {
                ctx.set_paint(SURFACE);
                ctx.fill_path(&Circle::new((x, y + 14.), 8.).to_path(0.1));
                ctx.set_paint(INK);
                ctx.fill_path(&Circle::new((x, y + 14.), 6.).to_path(0.1));
                text(&mut ctx, &mut res, &format!("frame on screen {:.0} ms", m.3), x + 10., y + 44., 11., INK, false);
            }
        }
        text(&mut ctx, &mut res, &format!("frame 1 ready {frame_done:.0}"), xp(frame_done) - 2., y + 44., 11., INK2, true);
    }
    // Legend (two columns) and the marker key.
    let ly = bottom + 64.;
    for (i, label) in LABELS.iter().enumerate() {
        let (lx, yy) = (40. + (i % 2) as f64 * 540., ly + (i / 2) as f64 * 20.);
        ctx.set_transform(Affine::IDENTITY);
        ctx.set_paint(SERIES[i]);
        ctx.fill_path(&RoundedRect::new(lx, yy - 10., lx + 14., yy + 2., 2.).to_path(0.1));
        text(&mut ctx, &mut res, label, lx + 20., yy, 12., INK2, false);
    }
    text(&mut ctx, &mut res, "diamond: window on screen (CGWindowList)   dot: first frame on screen (Metal presentedTime; GPUI: same probe in a patched copy)", 40., ly + 44., 12., INK2, false);
    ctx.flush();
    let mut pm = vello_cpu::Pixmap::new(w, h);
    ctx.render(&mut pm, &mut res);
    deka_ab::common::write_png(std::path::Path::new(output), w.into(), h.into(), pm.data_as_u8_slice());
    println!("wrote {output}");
}
