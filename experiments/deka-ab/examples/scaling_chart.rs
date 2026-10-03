//! Draw the fork's scaling curves (offscreen frame time vs strip threads) as a
//! PNG with vello_cpu. Input CSV: `machine,shapes,variant,threads,frame_ms`
//! (measured by `ab-hybrid --app scaling`). Small multiples: one panel per
//! (machine, canvas size), each on its own y axis; one line per variant
//! (validated categorical slots in fixed order); a muted 16.7 ms (60 fps) line.
//!
//!   cargo run --release --features cpu-backend --example scaling_chart -- data.csv out.png
//!
//! The phase-4 chart (one series per canvas size) was drawn from the
//! four-column form `machine,shapes,threads,frame_ms`, which still works: the
//! variant is then the canvas size.

use deka_ab::sparse::gfx::kurbo::{Affine, BezPath, Circle, Line, Rect, Shape, Stroke};
use deka_ab::sparse::gfx::peniko::Color;
use deka_ab::sparse::{Sparse, font, simple_glyphs};

const SURFACE: Color = Color::from_rgba8(0xfc, 0xfc, 0xfb, 0xff);
const INK: Color = Color::from_rgba8(0x0b, 0x0b, 0x0b, 0xff);
const INK2: Color = Color::from_rgba8(0x52, 0x51, 0x4e, 0xff);
const MUTED: Color = Color::from_rgba8(0x89, 0x87, 0x81, 0xff);
const GRID: Color = Color::from_rgba8(0xe1, 0xe0, 0xd9, 0xff);
const BASE: Color = Color::from_rgba8(0xc3, 0xc2, 0xb7, 0xff);
// Validated categorical slots 1 and 2 (dataviz reference palette).
const SERIES: [Color; 2] = [Color::from_rgba8(0x2a, 0x78, 0xd6, 0xff), Color::from_rgba8(0xeb, 0x68, 0x34, 0xff)];

#[allow(clippy::too_many_arguments, reason = "one call site per label; a struct would only rename the arguments")]
fn text(ctx: &mut vello_cpu::RenderContext, res: &mut vello_cpu::Resources, s: &str, x: f64, y: f64, size: f32, color: Color, right: bool) {
    let glyphs = simple_glyphs(s, size);
    let width = glyphs.last().map(|g| g.x as f64 + size as f64 * 0.55).unwrap_or(0.);
    ctx.set_transform(Affine::translate((if right { x - width } else { x }, y)));
    ctx.set_paint(color);
    Sparse::glyphs(ctx, res, &font(), size, &glyphs);
}

struct Row {
    machine: String,
    shapes: u32,
    variant: String,
    threads: u32,
    ms: f64,
}

fn push_unique<T: PartialEq + Clone>(v: &mut Vec<T>, x: &T) {
    if !v.contains(x) {
        v.push(x.clone());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(input), Some(output)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: scaling_chart data.csv out.png");
        return;
    };
    let Ok(csv) = std::fs::read_to_string(input) else {
        eprintln!("cannot read {input}");
        return;
    };
    // The machine label may contain commas: take the other fields from the right.
    let rows: Vec<Row> = csv
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let f: Vec<&str> = l.rsplitn(5, ',').map(str::trim).collect();
            let ms = f.first()?.parse().ok()?;
            let threads = f.get(1)?.parse().ok()?;
            // Five columns: ms, threads, variant, shapes, machine (reversed).
            if let (Some(variant), Some(Ok(shapes)), Some(machine)) = (f.get(2), f.get(3).map(|s| s.parse::<u32>()), f.get(4)) {
                return Some(Row { machine: machine.to_string(), shapes, variant: variant.to_string(), threads, ms });
            }
            // Four columns: ms, threads, shapes, machine-with-commas.
            let rest: Vec<&str> = l.rsplitn(4, ',').map(str::trim).collect();
            let shapes: u32 = rest.get(2)?.parse().ok()?;
            Some(Row { machine: rest.get(3)?.to_string(), shapes, variant: format!("canvas {}k", shapes / 1000), threads, ms })
        })
        .collect();
    let mut panels: Vec<(String, u32)> = vec![];
    let mut variants: Vec<String> = vec![];
    for r in &rows {
        push_unique(&mut panels, &(r.machine.clone(), r.shapes));
        push_unique(&mut variants, &r.variant);
    }
    let mut machines: Vec<String> = vec![];
    let mut sizes: Vec<u32> = vec![];
    for (m, s) in &panels {
        push_unique(&mut machines, m);
        push_unique(&mut sizes, s);
    }
    let (pw, ph) = (520.0f64, 330.0f64);
    let (w, h) = ((pw * machines.len() as f64 + 40.) as u16, (ph * sizes.len() as f64 + 110.) as u16);
    let mut ctx = vello_cpu::RenderContext::new(w, h);
    let mut res = vello_cpu::Resources::new();
    ctx.set_paint(SURFACE);
    ctx.fill_rect(&Rect::new(0., 0., w as f64, h as f64));
    text(&mut ctx, &mut res, "Forked vello_gpu: canvas frame time vs threads (offscreen 3200x2000; lower is better)", 24., 32., 16., INK, false);
    for (mi, machine) in machines.iter().enumerate() {
        for (si, &shapes) in sizes.iter().enumerate() {
            let panel: Vec<&Row> = rows.iter().filter(|r| &r.machine == machine && r.shapes == shapes).collect();
            if panel.is_empty() {
                continue;
            }
            let x0 = 24. + mi as f64 * pw + 50.;
            let top = 60. + si as f64 * ph;
            let (y0, y1) = (top + 34., top + ph - 50.);
            let plot_w = pw - 100.;
            let y_max = panel.iter().map(|r| r.ms).fold(0., f64::max).max(20.);
            let step = if y_max > 60. { 20. } else if y_max > 30. { 10. } else { 5. };
            let y_max = (y_max / step).ceil() * step;
            let mut threads: Vec<u32> = panel.iter().map(|r| r.threads).collect();
            threads.sort_unstable();
            threads.dedup();
            // x positions: log2 of thread count, so equal steps mean doubling.
            let lmax = (*threads.last().unwrap_or(&1) as f64).log2().max(1.);
            let xp = |t: u32| x0 + (t as f64).log2() / lmax * plot_w;
            let yp = |ms: f64| y1 - ms / y_max * (y1 - y0);
            text(&mut ctx, &mut res, &format!("{machine} - canvas {}k shapes", shapes / 1000), x0 - 40., top + 14., 13., INK, false);
            ctx.set_stroke(Stroke::new(1.));
            let mut v = 0.;
            while v <= y_max {
                ctx.set_transform(Affine::IDENTITY);
                ctx.set_paint(if v == 0. { BASE } else { GRID });
                ctx.stroke_path(&Line::new((x0, yp(v)), (x0 + plot_w, yp(v))).to_path(0.1));
                text(&mut ctx, &mut res, &format!("{v:.0}"), x0 - 8., yp(v) + 4., 11., MUTED, true);
                v += step;
            }
            text(&mut ctx, &mut res, "ms", x0 - 8., y0 - 12., 11., MUTED, true);
            if 16.7 < y_max {
                ctx.set_transform(Affine::IDENTITY);
                ctx.set_paint(MUTED);
                ctx.set_stroke(Stroke::new(1.).with_dashes(0., [4., 4.]));
                ctx.stroke_path(&Line::new((x0, yp(16.7)), (x0 + plot_w, yp(16.7))).to_path(0.1));
            }
            for &t in &threads {
                text(&mut ctx, &mut res, &t.to_string(), xp(t) - 4., y1 + 18., 11., MUTED, false);
            }
            text(&mut ctx, &mut res, "threads", x0 + plot_w / 2. - 24., y1 + 36., 12., INK2, false);
            for (vi, variant) in variants.iter().enumerate().take(SERIES.len()) {
                let mut pts: Vec<(u32, f64)> = panel.iter().filter(|r| &r.variant == variant).map(|r| (r.threads, r.ms)).collect();
                pts.sort_by_key(|p| p.0);
                if pts.is_empty() {
                    continue;
                }
                let mut path = BezPath::new();
                for (i, (t, ms)) in pts.iter().enumerate() {
                    if i == 0 { path.move_to((xp(*t), yp(*ms))) } else { path.line_to((xp(*t), yp(*ms))) }
                }
                ctx.set_transform(Affine::IDENTITY);
                ctx.set_paint(SERIES[vi]);
                ctx.set_stroke(Stroke::new(2.));
                ctx.stroke_path(&path);
                for (t, ms) in &pts {
                    ctx.set_paint(SURFACE);
                    ctx.fill_path(&Circle::new((xp(*t), yp(*ms)), 6.).to_path(0.1));
                    ctx.set_paint(SERIES[vi]);
                    ctx.fill_path(&Circle::new((xp(*t), yp(*ms)), 4.).to_path(0.1));
                }
                // Direct labels: first and last value.
                let (t0, ms0) = pts[0];
                let (tn, msn) = pts[pts.len() - 1];
                // First series labelled above its first point, the second below.
                let dy = if vi == 0 { -8. } else { 16. };
                text(&mut ctx, &mut res, &format!("{ms0:.0}"), xp(t0) + 8., yp(ms0) + dy, 11., INK2, false);
                text(&mut ctx, &mut res, &format!("{msn:.0} ms"), xp(tn) + 8., yp(msn) + 4., 11., INK2, false);
            }
        }
    }
    let ly = h as f64 - 20.;
    for (vi, label) in variants.iter().enumerate().take(SERIES.len()) {
        let lx = 74. + vi as f64 * 260.;
        ctx.set_transform(Affine::IDENTITY);
        ctx.set_paint(SERIES[vi]);
        ctx.fill_path(&Circle::new((lx, ly - 4.), 5.).to_path(0.1));
        text(&mut ctx, &mut res, label, lx + 12., ly, 12., INK2, false);
    }
    // The dashed reference line is explained once, in the legend.
    let lx = 74. + variants.len().min(SERIES.len()) as f64 * 260.;
    ctx.set_transform(Affine::IDENTITY);
    ctx.set_paint(MUTED);
    ctx.set_stroke(Stroke::new(1.).with_dashes(0., [4., 4.]));
    ctx.stroke_path(&Line::new((lx - 6., ly - 4.), (lx + 18., ly - 4.)).to_path(0.1));
    text(&mut ctx, &mut res, "16.7 ms = 60 fps", lx + 26., ly, 12., INK2, false);
    ctx.flush();
    let mut pm = vello_cpu::Pixmap::new(w, h);
    ctx.render(&mut pm, &mut res);
    deka_ab::common::write_png(std::path::Path::new(output), w.into(), h.into(), pm.data_as_u8_slice());
    println!("wrote {output}");
}
