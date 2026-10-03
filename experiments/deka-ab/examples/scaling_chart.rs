//! Draw the phase-4 scaling curve (offscreen frame time vs strip threads) as a
//! PNG with vello_cpu. Input CSV: `machine,shapes,threads,frame_ms` (measured
//! by `ab-hybrid --app scaling`). One panel per machine (small multiples on a
//! shared y axis); one line per canvas size; a muted 16.7 ms (60 fps) line.
//!
//!   cargo run --release --features cpu-backend --example scaling_chart -- data.csv out.png

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
    // (machine, shapes, threads, ms)
    let rows: Vec<(String, u32, u32, f64)> = csv
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            // The machine label may contain commas: take the numbers from the right.
            let f: Vec<&str> = l.rsplitn(4, ',').map(str::trim).collect();
            Some((f.get(3)?.to_string(), f.get(2)?.parse().ok()?, f.get(1)?.parse().ok()?, f.first()?.parse().ok()?))
        })
        .collect();
    let mut machines: Vec<String> = vec![];
    for r in &rows {
        if !machines.contains(&r.0) {
            machines.push(r.0.clone());
        }
    }
    let (pw, ph) = (560.0f64, 360.0f64);
    let (w, h) = ((pw * machines.len() as f64 + 40.) as u16, (ph + 120.) as u16);
    let mut ctx = vello_cpu::RenderContext::new(w, h);
    let mut res = vello_cpu::Resources::new();
    ctx.set_paint(SURFACE);
    ctx.fill_rect(&Rect::new(0., 0., w as f64, h as f64));
    text(&mut ctx, &mut res, "Forked vello_gpu: canvas frame time vs strip threads (offscreen 3200x2000; lower is better)", 24., 32., 16., INK, false);
    let y_max = rows.iter().map(|r| r.3).fold(0., f64::max).max(20.).ceil();
    let y_max = (y_max / 10.).ceil() * 10.;
    for (mi, machine) in machines.iter().enumerate() {
        let x0 = 24. + mi as f64 * pw + 50.;
        let (y0, y1) = (84., 84. + ph - 70.);
        let plot_w = pw - 90.;
        let threads: Vec<u32> = {
            let mut t: Vec<u32> = rows.iter().filter(|r| &r.0 == machine).map(|r| r.2).collect();
            t.sort_unstable();
            t.dedup();
            t
        };
        // x positions: log2 of thread count, so equal steps mean doubling.
        let lmax = (*threads.last().unwrap_or(&1) as f64).log2().max(1.);
        let xp = |t: u32| x0 + (t as f64).log2() / lmax * plot_w;
        let yp = |ms: f64| y1 - ms / y_max * (y1 - y0);
        text(&mut ctx, &mut res, machine, x0 - 40., 60., 13., INK, false);
        // gridlines + y labels
        ctx.set_transform(Affine::IDENTITY);
        ctx.set_stroke(Stroke::new(1.));
        let step = if y_max > 60. { 20. } else { 10. };
        let mut v = 0.;
        while v <= y_max {
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(if v == 0. { BASE } else { GRID });
            ctx.stroke_path(&Line::new((x0, yp(v)), (x0 + plot_w, yp(v))).to_path(0.1));
            text(&mut ctx, &mut res, &format!("{v:.0}"), x0 - 8., yp(v) + 4., 11., MUTED, true);
            v += step;
        }
        text(&mut ctx, &mut res, "ms", x0 - 8., y0 - 10., 11., MUTED, true);
        // 60 fps reference
        if 16.7 < y_max {
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(MUTED);
            ctx.set_stroke(Stroke::new(1.).with_dashes(0., [4., 4.]));
            ctx.stroke_path(&Line::new((x0, yp(16.7)), (x0 + plot_w, yp(16.7))).to_path(0.1));
            text(&mut ctx, &mut res, "16.7 ms = 60 fps", x0 + plot_w, yp(16.7) - 6., 11., MUTED, true);
        }
        // x labels
        for &t in &threads {
            text(&mut ctx, &mut res, &t.to_string(), xp(t) - 4., y1 + 18., 11., MUTED, false);
        }
        text(&mut ctx, &mut res, "strip threads", x0 + plot_w / 2. - 40., y1 + 38., 12., INK2, false);
        // series
        for (si, shapes) in [10_000u32, 1_000].iter().enumerate() {
            let mut pts: Vec<(u32, f64)> = rows.iter().filter(|r| &r.0 == machine && r.1 == *shapes).map(|r| (r.2, r.3)).collect();
            pts.sort_by_key(|p| p.0);
            if pts.is_empty() {
                continue;
            }
            let mut path = BezPath::new();
            for (i, (t, ms)) in pts.iter().enumerate() {
                if i == 0 { path.move_to((xp(*t), yp(*ms))) } else { path.line_to((xp(*t), yp(*ms))) }
            }
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(SERIES[si]);
            ctx.set_stroke(Stroke::new(2.));
            ctx.stroke_path(&path);
            for (t, ms) in &pts {
                ctx.set_paint(SURFACE);
                ctx.fill_path(&Circle::new((xp(*t), yp(*ms)), 6.).to_path(0.1));
                ctx.set_paint(SERIES[si]);
                ctx.fill_path(&Circle::new((xp(*t), yp(*ms)), 4.).to_path(0.1));
            }
            // direct labels: first and last value
            let (t0, ms0) = pts[0];
            let (tn, msn) = pts[pts.len() - 1];
            text(&mut ctx, &mut res, &format!("{ms0:.0}"), xp(t0) + 8., yp(ms0) - 6., 11., INK2, false);
            text(&mut ctx, &mut res, &format!("{msn:.0} ms"), xp(tn) + 8., yp(msn) + 4., 11., INK2, false);
        }
    }
    // legend
    let ly = h as f64 - 22.;
    for (si, label) in ["canvas 10k shapes", "canvas 1k shapes (with physics)"].iter().enumerate() {
        let lx = 74. + si as f64 * 220.;
        ctx.set_transform(Affine::IDENTITY);
        ctx.set_paint(SERIES[si]);
        ctx.fill_path(&Circle::new((lx, ly - 4.), 5.).to_path(0.1));
        text(&mut ctx, &mut res, label, lx + 12., ly, 12., INK2, false);
    }
    ctx.flush();
    let mut pm = vello_cpu::Pixmap::new(w, h);
    ctx.render(&mut pm, &mut res);
    deka_ab::common::write_png(std::path::Path::new(output), w.into(), h.into(), pm.data_as_u8_slice());
    println!("wrote {output}");
}
