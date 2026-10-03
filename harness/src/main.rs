//! Evidence harness for deka_native_ui text. Uses only deka_native_ui's public API, so the
//! same source builds against main (fontdue) and the parley branch.
//!
//!   sheet <dir>   offscreen PNGs (scene composited on the CPU) + layout bounds JSON
//!   bench         headless per-frame CPU cost of scene rendering
//!   window        GPUI window: start-to-first-frame, idle CPU over 10 s, idle memory; exits itself
//!   world         GPUI portfolio world: CPU per second while animating for 10 s; exits itself
use deka_native_ui::{
    Application, Edges, Length, Node, Style,
    scene::{Renderer, Scene},
};
use std::time::{Duration, Instant};

const SHEET_W: f32 = 640.;
const SHEET_H: f32 = 560.;
const PARAGRAPH: &str = "Deka draws its own text: shaping, line breaking and fallback all happen before the GPU sees a single glyph, so the same layout ships to the desktop and the browser.";

fn node(style: Style, text: Option<&str>, children: Vec<Node>) -> Node {
    Node {
        id: String::new(),
        style,
        text: text.map(str::to_owned),
        on_click: None,
        children,
    }
}
fn text(t: &str, size: f32, nowrap: bool) -> Node {
    node(
        Style {
            font_size: Some(size),
            nowrap: Some(nowrap),
            ..Default::default()
        },
        Some(t),
        vec![],
    )
}
fn boxed(width: f32, background: u32, color: u32, children: Vec<Node>) -> Node {
    node(
        Style {
            width: Length::Px(width),
            padding: Edges::all(8.),
            background: Some(background),
            color: Some(color),
            align: deka_native_ui::Align::Start,
            gap_y: 4.,
            ..Default::default()
        },
        None,
        children,
    )
}
/// The text sheet: every size the brief names, wrapped paragraphs, Japanese, emoji.
pub fn sheet() -> Node {
    sheet_with(true)
}
/// `scripts: false` leaves out the Japanese and emoji rows (Latin-only application).
pub fn sheet_with(scripts: bool) -> Node {
    let mut rows = vec![];
    for size in [11., 12., 14., 16., 24.] {
        rows.push(text(
            &format!("{size} px  The quick brown fox jumps over the lazy dog. 0123456789 Hamburgefonstiv"),
            size,
            true,
        ));
    }
    rows.push(node(
        Style {
            row: true,
            gap_x: 12.,
            align: deka_native_ui::Align::Start,
            ..Default::default()
        },
        None,
        vec![
            boxed(300., 0xf3efe3, 0x1a1611, vec![text(PARAGRAPH, 14., false)]),
            boxed(220., 0x1e1610, 0xf9edcf, vec![text(PARAGRAPH, 12., false)]),
        ],
    ));
    if scripts {
        rows.push(text("日本語: こんにちは、世界。ネイティブ UI の文字表示", 16., true));
        rows.push(text("Emoji: 🎉 🚀 👍 ❤️ 🇨🇦 done", 16., true));
        rows.push(text("Mixed 12 px: café naïve – “quotes” → arrows, ½ ≠ ≈", 12., true));
    }
    // Each row is its own element: adjacent bare text with equal style would merge into one run.
    let rows = rows
        .into_iter()
        .map(|row| node(Style::default(), None, vec![row]))
        .collect();
    node(
        Style {
            padding: Edges::all(16.),
            gap_y: 10.,
            align: deka_native_ui::Align::Start,
            ..Default::default()
        },
        None,
        rows,
    )
}

/// Same strings, sizes, colour and spacing as the spike's CoreText reference sheet (980x190).
fn reference_sheet() -> Node {
    const SAMPLE: &str =
        "Hamburgefonstiv 0123456789 — The quick brown fox jumps over the lazy dog.";
    let rows = [11., 12., 14., 16., 24.]
        .iter()
        .map(|size| {
            node(
                Style::default(),
                None,
                vec![text(&format!("{size}px  {SAMPLE}"), *size, true)],
            )
        })
        .collect();
    node(
        Style {
            padding: Edges {
                top: 12.,
                left: 12.,
                ..Default::default()
            },
            gap_y: 10.,
            color: Some(0x111111),
            align: deka_native_ui::Align::Start,
            ..Default::default()
        },
        None,
        rows,
    )
}

// ---------- CPU compositor (what both adapters draw: quads + RGBA images) ----------
fn composite(scene: &Scene, scale: f32) -> (u32, u32, Vec<u8>) {
    let w = (scene.width * scale).round() as usize;
    let h = (scene.height * scale).round() as usize;
    let bg = scene.background;
    let mut px: Vec<u8> = (0..w * h)
        .flat_map(|_| [(bg >> 16) as u8, (bg >> 8) as u8, bg as u8])
        .collect();
    let images: std::collections::HashMap<_, _> =
        scene.images.iter().map(|i| (i.id.as_str(), i)).collect();
    for p in &scene.paint {
        if p.opacity <= 0. {
            continue;
        }
        let Some(r) = p.rect.intersection(p.clip) else {
            continue;
        };
        let x0 = (r.x * scale).floor().max(0.) as usize;
        let y0 = (r.y * scale).floor().max(0.) as usize;
        let x1 = (((r.x + r.width) * scale).ceil() as usize).min(w);
        let y1 = (((r.y + r.height) * scale).ceil() as usize).min(h);
        let image = p.image.as_deref().and_then(|id| images.get(id));
        for y in y0..y1 {
            for x in x0..x1 {
                let lx = (x as f32 + 0.5) / scale;
                let ly = (y as f32 + 0.5) / scale;
                if !p.rect.contains(lx, ly) || !p.clip.contains(lx, ly) {
                    continue;
                }
                let (rgb, a) = if let Some(img) = image {
                    let u = ((lx - p.rect.x) / p.rect.width * img.width as f32) as usize;
                    let v = ((ly - p.rect.y) / p.rect.height * img.height as f32) as usize;
                    let i = (v.min(img.height - 1) * img.width + u.min(img.width - 1)) * 4;
                    let s = &img.rgba[i..i + 4];
                    ([s[0], s[1], s[2]], s[3] as f32 / 255.)
                } else {
                    let rr = p.radius.min(p.rect.width / 2.).min(p.rect.height / 2.);
                    let dx = (lx - (p.rect.x + p.rect.width / 2.)).abs() - (p.rect.width / 2. - rr);
                    let dy =
                        (ly - (p.rect.y + p.rect.height / 2.)).abs() - (p.rect.height / 2. - rr);
                    let d = dx.max(0.).hypot(dy.max(0.)) + dx.max(dy).min(0.) - rr;
                    (
                        [(p.color >> 16) as u8, (p.color >> 8) as u8, p.color as u8],
                        (0.5 - d * scale).clamp(0., 1.),
                    )
                };
                let a = a * p.opacity;
                let o = (y * w + x) * 3;
                for c in 0..3 {
                    px[o + c] = (rgb[c] as f32 * a + px[o + c] as f32 * (1. - a)).round() as u8;
                }
            }
        }
    }
    (w as u32, h as u32, px)
}
fn write_png(path: &std::path::Path, (w, h, px): (u32, u32, Vec<u8>)) {
    let file = std::fs::File::create(path).expect("png file");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(&px)
        .expect("png data");
}
fn bounds(scene: &Scene) -> serde_json::Value {
    serde_json::Value::Array(
        scene
            .nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "text": n.text, "x": n.layout_rect.x, "y": n.layout_rect.y,
                    "w": n.layout_rect.width, "h": n.layout_rect.height
                })
            })
            .collect(),
    )
}
const WEB_SOURCE: &str = r#"export fn App() {
    let count = 0;
    return (<view className="p-4 gap-2"><p>{count}</p>
        <button onClick={fn() { count += 1; }}>Add one</button>
        <p className="text-sm">A longer line that has to wrap inside the preview once the window is narrow enough to force it.</p>
    </view>);
}"#;
fn vm_scene(source: &str, entry: &str, width: f32, scale: f32) -> Scene {
    let program =
        deka_vm::compiler::compile_entry(source, &deka_vm::Hosts::default(), entry).unwrap();
    let app = deka_vm::ui::VmApp::new(program).unwrap();
    let host = deka_native_ui::Host::new(app);
    Renderer::new().render(&host.render(), width, 300., scale)
}
fn sheet_cmd(dir: &str) {
    let dir = std::path::Path::new(dir);
    std::fs::create_dir_all(dir).unwrap();
    let counter = std::fs::read_to_string("../deka/crates/deka_vm/examples/counter.dsx").unwrap();
    let mut layout = serde_json::Map::new();
    for scale in [1., 2.] {
        let s = Renderer::new().render(&sheet(), SHEET_W, SHEET_H, scale);
        write_png(&dir.join(format!("sheet-{scale}x.png")), composite(&s, scale));
        let c = vm_scene(&counter, "Counter", 560., scale);
        write_png(&dir.join(format!("counter-{scale}x.png")), composite(&c, scale));
        if scale == 1. {
            layout.insert("sheet".into(), bounds(&s));
            layout.insert("counter".into(), bounds(&c));
            for width in [560., 240.] {
                let web = vm_scene(WEB_SOURCE, "App", width, 1.);
                layout.insert(format!("web-sample-{width}"), bounds(&web));
            }
        }
    }
    for scale in [1., 2.] {
        let s = Renderer::new().render(&reference_sheet(), 980., 190., scale);
        write_png(&dir.join(format!("reference-{scale}x.png")), composite(&s, scale));
    }
    // The world draws labels at 2x internally; composite at 2x like a Retina window.
    let mut world = deka_native_ui::world::World::new();
    world.start();
    let scene = world.frame(960., 640., 0., true);
    write_png(&dir.join("world-2x.png"), composite(&scene, 2.));
    std::fs::write(
        dir.join("layout.json"),
        serde_json::to_string_pretty(&layout).unwrap(),
    )
    .unwrap();
    println!("wrote {}", dir.display());
}

fn stats(mut v: Vec<f64>) -> String {
    v.sort_by(f64::total_cmp);
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    format!(
        "mean {mean:.3} ms  p50 {:.3}  p95 {:.3}  max {:.3}",
        v[v.len() / 2],
        v[v.len() * 95 / 100],
        v[v.len() - 1]
    )
}
fn bench_cmd() {
    let t = Instant::now();
    let renderer = Renderer::new();
    let created = t.elapsed();
    let first = Instant::now();
    let scene = renderer.render(&sheet_with(false), SHEET_W, SHEET_H, 2.);
    let first = first.elapsed();
    println!(
        "renderer new {:.3} ms; first Latin-only sheet frame @2x {:.3} ms ({} paints, {} images)",
        created.as_secs_f64() * 1e3,
        first.as_secs_f64() * 1e3,
        scene.paint.len(),
        scene.images.len()
    );
    let root = sheet();
    let first = Instant::now();
    let scene = renderer.render(&root, SHEET_W, SHEET_H, 2.);
    println!(
        "first full sheet frame (Japanese + emoji added) @2x {:.3} ms ({} paints, {} images)",
        first.elapsed().as_secs_f64() * 1e3,
        scene.paint.len(),
        scene.images.len()
    );
    let mut v = vec![];
    for _ in 0..300 {
        let t = Instant::now();
        std::hint::black_box(renderer.render(&root, SHEET_W, SHEET_H, 2.));
        v.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("sheet @2x steady frame: {}", stats(v));
    let mut world = deka_native_ui::world::World::new();
    world.start();
    let mut v = vec![];
    for i in 0..900 {
        let t = Instant::now();
        std::hint::black_box(world.frame(960., 640., i as f64 * 1000. / 60., false));
        if i >= 60 {
            v.push(t.elapsed().as_secs_f64() * 1e3);
        }
    }
    println!("world frame (animating): {}", stats(v));
}

// ---------- process metrics (macOS) ----------
fn cpu_seconds() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let t = |tv: libc::timeval| tv.tv_sec as f64 + tv.tv_usec as f64 / 1e6;
    t(u.ru_utime) + t(u.ru_stime)
}
fn memory_mb() -> (f64, f64) {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            &mut info as *mut _ as *mut libc::rusage_info_t,
        )
    };
    assert_eq!(rc, 0, "proc_pid_rusage");
    (
        info.ri_phys_footprint as f64 / 1048576.,
        info.ri_resident_size as f64 / 1048576.,
    )
}
fn process_start() -> std::time::SystemTime {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let rc = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    assert_eq!(rc, size, "proc_pidinfo");
    std::time::UNIX_EPOCH
        + Duration::from_secs(info.pbi_start_tvsec)
        + Duration::from_micros(info.pbi_start_tvusec)
}

struct SheetApp {
    renders: std::cell::Cell<u32>,
    scripts: bool,
}
impl Application for SheetApp {
    fn initial_state(&self) -> Vec<f64> {
        vec![]
    }
    fn render(&self, _: &[f64]) -> Node {
        let n = self.renders.get() + 1;
        self.renders.set(n);
        if n == 2 {
            // The first frame requested an animation frame; this call follows its present.
            let first = process_start().elapsed().unwrap();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                let cpu = cpu_seconds();
                std::thread::sleep(Duration::from_secs(10));
                let idle = cpu_seconds() - cpu;
                let (footprint, resident) = memory_mb();
                println!(
                    "{}",
                    serde_json::json!({
                        "start_to_first_frame_ms": first.as_secs_f64() * 1e3,
                        "idle_cpu_ms_over_10s": idle * 1e3,
                        "idle_footprint_mb": footprint, "idle_resident_mb": resident
                    })
                );
                std::process::exit(0);
            });
        }
        let mut root = sheet_with(self.scripts);
        // One short fade so the host requests exactly the frames after the first present.
        root.style.motion.enter = 1;
        root.style.duration_ms = 150.;
        root
    }
    fn event(&self, _: usize, _: &mut [f64]) {}
}
fn world_cmd() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(3));
        let cpu = cpu_seconds();
        std::thread::sleep(Duration::from_secs(10));
        let used = cpu_seconds() - cpu;
        let (footprint, resident) = memory_mb();
        println!(
            "{}",
            serde_json::json!({
                "world_cpu_ms_per_s": used * 1e3 / 10.,
                "footprint_mb": footprint, "resident_mb": resident
            })
        );
        std::process::exit(0);
    });
    deka_native_ui::world::run();
}

fn read_png(path: &str) -> (usize, usize, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    if info.color_type == png::ColorType::Rgba {
        buf = buf
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as f32 / 255.;
                [0, 1, 2].map(|c| (p[c] as f32 * a + 255. * (1. - a)).round() as u8)
            })
            .collect();
    } else {
        assert_eq!(info.color_type, png::ColorType::Rgb);
    }
    (info.width as usize, info.height as usize, buf)
}
/// Before above after, separated by a red bar; optional crop and nearest-neighbour zoom.
fn compare_cmd(paths: &[String], out: &str, crop: Option<[usize; 5]>) {
    let imgs: Vec<_> = paths.iter().map(|p| read_png(p)).collect();
    let [x, y, w, h, z] = crop.unwrap_or([0, 0, imgs[0].0, imgs[0].1, 1]);
    let bar = 4 * z.min(2);
    let n = imgs.len();
    let (ow, oh) = (w * z, h * z * n + bar * (n - 1));
    let mut px = vec![0u8; ow * oh * 3];
    for (o, c) in px.chunks_exact_mut(3).enumerate() {
        if (o / ow) % (h * z + bar) >= h * z {
            c.copy_from_slice(&[220, 40, 40]);
        }
    }
    for (k, (iw, ih, data)) in imgs.iter().enumerate() {
        let top = k * (h * z + bar);
        for oy in 0..h * z {
            for ox in 0..w * z {
                let (sx, sy) = (x + ox / z, y + oy / z);
                let s = if sx < *iw && sy < *ih {
                    &data[(sy * iw + sx) * 3..][..3]
                } else {
                    &[255, 255, 255][..]
                };
                let o = ((top + oy) * ow + ox) * 3;
                px[o..o + 3].copy_from_slice(s);
            }
        }
    }
    write_png(std::path::Path::new(out), (ow as u32, oh as u32, px));
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("watchdog: 60 s elapsed");
        std::process::exit(2);
    });
    match args.get(1).map(String::as_str) {
        Some("sheet") => sheet_cmd(args.get(2).map(String::as_str).unwrap_or("out")),
        Some("bench") => bench_cmd(),
        Some("window") => deka_native_ui::run(SheetApp {
            renders: Default::default(),
            scripts: args.get(2).map(String::as_str) == Some("full"),
        }),
        Some("world") => world_cmd(),
        // Total ink (sum of darkness) per image: a weight comparison between renderers.
        Some("ink") => {
            for p in &args[2..] {
                let (_, _, d) = read_png(p);
                let ink: f64 = d.iter().map(|v| (255 - *v) as f64 / 255. / 3.).sum();
                println!("{ink:>10.0}  {p}");
            }
        }
        // compare <out.png> <x,y,w,h,zoom | full> <image>...
        Some("compare") => compare_cmd(
            &args[4..],
            &args[2],
            (args[3] != "full").then(|| {
                let v: Vec<usize> = args[3].split(',').map(|n| n.parse().unwrap()).collect();
                [v[0], v[1], v[2], v[3], v[4]]
            }),
        ),
        _ => eprintln!("usage: parley-evidence sheet <dir> | bench | window | world"),
    }
}
