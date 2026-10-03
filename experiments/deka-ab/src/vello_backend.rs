//! Backend B: deka_native_ui's Scene drawn by vello in a winit window, on wgpu.
//! No GPUI. Text: deka's Scene still carries fontdue glyph bitmaps (same as
//! backend A); parley + vello glyph rendering is exercised by the text sheet
//! and the editor.

#[path = "../../vello-gpui/src/graph.rs"]
#[allow(dead_code, reason = "shared with the phase-1 crate, which also uses the Hairball layout")]
mod graph;
/// What graph.rs draws with.
mod gfx {
    pub use vello::{Scene, kurbo, peniko};
}

use crate::common::{self, App as AppKind, Args, Protocol, Step, WINDOW_H, WINDOW_W};
use deka_native_ui::scene::{GlyphImage, Scene as DekaScene};
use deka_native_ui::{Host, world::World};
use parley::{FontContext, LayoutContext, PlainEditor, PositionedLayoutItem, StyleProperty};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vello::kurbo::{Affine, Rect, RoundedRect, Stroke};
use vello::peniko::{Blob, Color, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

fn rgb(c: u32, alpha: f32) -> Color {
    Color::from_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, (alpha.clamp(0., 1.) * 255.).round() as u8)
}

// ---------------------------------------------------------------------------
// deka Scene -> vello Scene

/// Translates deka_native_ui's Scene (rects, rounded rects, RGBA images incl.
/// fontdue glyph bitmaps) into a vello Scene. Images are cached by deka's id so
/// vello uploads each once; unused ids are evicted, like the GPUI glue does.
#[derive(Default)]
pub struct DekaVello {
    images: HashMap<String, ImageData>,
}

impl DekaVello {
    pub fn encode(&mut self, deka: &DekaScene, scale: f64, out: &mut Scene) {
        out.reset();
        let s = Affine::scale(scale);
        out.fill(Fill::NonZero, s, rgb(deka.background, 1.), None, &Rect::new(0., 0., deka.width as f64, deka.height as f64));
        let lookup: HashMap<&str, &GlyphImage> = deka.images.iter().map(|g| (g.id.as_str(), g)).collect();
        let mut used = HashSet::new();
        // A vello clip layer costs tiles for its whole clip rect, so share one
        // layer across consecutive paints with the same clip, and open none for
        // paints already inside their clip. (One layer per primitive, the way
        // GPUI content masks work, overflowed vello's tile buffer on a static page.)
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
                    out.pop_layer();
                }
                out.push_clip_layer(Fill::NonZero, s, &c);
                active_clip = Some(c);
            } else if inside && let Some(a) = active_clip
                && !(r.x0 >= a.x0 && r.y0 >= a.y0 && r.x1 <= a.x1 && r.y1 <= a.y1)
            {
                // Inside its own clip but not the active one: close the layer.
                out.pop_layer();
                active_clip = None;
            }
            if let Some(id) = &p.image {
                if let Some(g) = lookup.get(id.as_str()) {
                    used.insert(id.clone());
                    let image = self.images.entry(id.clone()).or_insert_with(|| ImageData {
                        data: Blob::new(Arc::new(g.rgba.clone())),
                        format: ImageFormat::Rgba8,
                        alpha_type: ImageAlphaType::Alpha,
                        width: g.width as u32,
                        height: g.height as u32,
                    });
                    // Glyph bitmaps are rasterised at device scale: snap to device pixels.
                    let (dx, dy) = ((r.x0 * scale).round(), (r.y0 * scale).round());
                    let sx = r.width() * scale / g.width.max(1) as f64;
                    let sy = r.height() * scale / g.height.max(1) as f64;
                    out.draw_image(
                        &ImageBrush::new(image.clone()).with_alpha(p.opacity),
                        Affine::translate((dx, dy)) * Affine::scale_non_uniform(sx, sy),
                    );
                }
            } else {
                out.fill(Fill::NonZero, s, rgb(p.color, p.opacity), None, &RoundedRect::from_rect(r, p.radius as f64));
            }
        }
        if active_clip.is_some() {
            out.pop_layer();
        }
        self.images.retain(|id, _| used.contains(id));
    }
}

// ---------------------------------------------------------------------------
// GPU

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
}

#[derive(Default, Clone, Copy)]
struct StartupTimes {
    gpu_after_device_mb: f64,
    gpu_after_renderer_mb: f64,
    window_ms: f64,
    adapter_device_ms: f64,
    vello_renderer_ms: f64,
}

fn new_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

fn new_gpu(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, times: &mut StartupTimes) -> Result<Gpu, String> {
    let t = Instant::now();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: surface,
        ..Default::default()
    }))
    .map_err(|e| format!("adapter: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("deka-ab"),
        required_features: adapter.features() & wgpu::Features::CLEAR_TEXTURE,
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }))
    .map_err(|e| format!("device: {e}"))?;
    times.adapter_device_ms = t.elapsed().as_secs_f64() * 1000.;
    times.gpu_after_device_mb = common::gpu_mb();
    if adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
        println!("[vello] adapter supports PIPELINE_CACHE");
    } else {
        println!("[vello] adapter does NOT support wgpu PIPELINE_CACHE (Metal backend); vello pipeline_cache cannot help here");
    }
    let t = Instant::now();
    let renderer = Renderer::new(&device, RendererOptions { antialiasing_support: AaSupport::area_only(), ..Default::default() })
        .map_err(|e| format!("vello renderer: {e}"))?;
    times.vello_renderer_ms = t.elapsed().as_secs_f64() * 1000.;
    times.gpu_after_renderer_mb = common::gpu_mb();
    Ok(Gpu { device, queue, renderer })
}


struct Target {
    view: wgpu::TextureView,
    texture: wgpu::Texture,
    width: u32,
    height: u32,
}

fn new_target(device: &wgpu::Device, width: u32, height: u32) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vello target"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    Target { view: texture.create_view(&Default::default()), texture, width, height }
}

fn render(gpu: &mut Gpu, scene: &Scene, target: &Target, base: Color) -> Result<(), String> {
    gpu.renderer
        .render_to_texture(&gpu.device, &gpu.queue, scene, &target.view, &RenderParams {
            base_color: base,
            width: target.width,
            height: target.height,
            antialiasing_method: AaConfig::Area,
        })
        .map_err(|e| format!("vello render: {e}"))
}

fn read_rgba(gpu: &Gpu, target: &Target) -> Vec<u8> {
    let padded = (target.width * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * target.height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        target.texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
        },
        wgpu::Extent3d { width: target.width, height: target.height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    if let Err(e) = gpu.device.poll(wgpu::PollType::wait_indefinitely()) {
        eprintln!("poll: {e}");
    }
    let mut out = Vec::with_capacity((target.width * target.height * 4) as usize);
    if let Ok(data) = buffer.slice(..).get_mapped_range() {
        for row in data.chunks_exact(padded as usize) {
            out.extend_from_slice(&row[..(target.width * 4) as usize]);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// parley text

/// parley Brush: an RGBA8 colour.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rgba(pub [u8; 4]);

pub fn register_font(font_cx: &mut FontContext) -> String {
    let families = font_cx.collection.register_fonts(Blob::new(Arc::new(common::FONT_BYTES)), None);
    families
        .first()
        .and_then(|(id, _)| font_cx.collection.family_name(*id).map(str::to_owned))
        .unwrap_or_else(|| "Atkinson Hyperlegible".into())
}

/// Draw a parley layout (positions in device px) with vello glyph runs.
pub fn draw_layout(scene: &mut Scene, layout: &parley::Layout<Rgba>, origin: (f64, f64), hint: bool) {
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
            let run = glyph_run.run();
            let [r, g, b, a] = glyph_run.style().brush.0;
            let skew = run.synthesis().skew().map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.));
            scene
                .draw_glyphs(run.font())
                .brush(Color::from_rgba8(r, g, b, a))
                .hint(hint)
                .transform(Affine::translate(origin))
                .glyph_transform(skew)
                .font_size(run.font_size())
                .normalized_coords(run.normalized_coords())
                .draw(
                    Fill::NonZero,
                    glyph_run.positioned_glyphs().map(|g| vello::Glyph { id: g.id, x: g.x, y: g.y }),
                );
        }
    }
}

fn text_sheets() {
    let mut times = StartupTimes::default();
    let instance = new_instance();
    let mut gpu = match new_gpu(&instance, None, &mut times) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let mut font_cx = FontContext::new();
    let mut layout_cx: LayoutContext<Rgba> = LayoutContext::new();
    let family = register_font(&mut font_cx);
    let deka_renderer = deka_native_ui::scene::Renderer::new();
    let root = common::text_sheet_root();
    for scale in [1u32, 2] {
        let (w, h) = (common::SHEET_W as u32 * scale, common::SHEET_H as u32 * scale);
        let target = new_target(&gpu.device, w, h);
        // (a) deka today: taffy + fontdue bitmaps, drawn by vello (GPUI draws the same bitmaps).
        let deka = deka_renderer.render(&root, common::SHEET_W, common::SHEET_H, scale as f32);
        let mut scene = Scene::new();
        DekaVello::default().encode(&deka, scale as f64, &mut scene);
        if render(&mut gpu, &scene, &target, Color::WHITE).is_ok() {
            common::write_png(&common::shots_dir().join(format!("text-deka-fontdue-{scale}x.png")), w, h, &read_rgba(&gpu, &target));
        }
        // (b, c) parley layout + vello glyph outlines, hinted and unhinted.
        for hint in [true, false] {
            let mut scene = Scene::new();
            scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None, &Rect::new(0., 0., w as f64, h as f64));
            let mut y = 12.0 * scale as f64;
            for size in common::TEXT_SIZES {
                let line = format!("{size}px  {}", common::TEXT_SAMPLE);
                let mut builder = layout_cx.ranged_builder(&mut font_cx, &line, scale as f32, true);
                builder.push_default(StyleProperty::FontSize(size));
                builder.push_default(StyleProperty::FontFamily(parley::FontFamily::Single(parley::FontFamilyName::Named(family.clone().into()))));
                builder.push_default(StyleProperty::Brush(Rgba([0x11, 0x11, 0x11, 0xff])));
                let mut layout = builder.build(&line);
                layout.break_all_lines(None);
                draw_layout(&mut scene, &layout, ((12 * scale) as f64, y.round()), hint);
                y += layout.height() as f64 + 10.0 * scale as f64;
            }
            if render(&mut gpu, &scene, &target, Color::WHITE).is_ok() {
                let name = if hint { "parley-vello-hinted" } else { "parley-vello-unhinted" };
                common::write_png(&common::shots_dir().join(format!("text-{name}-{scale}x.png")), w, h, &read_rgba(&gpu, &target));
            }
        }
    }
    println!("[vello] wrote text sheets (deka fontdue, parley+vello hinted/unhinted; 1x, 2x)");
    // Side by side (top to bottom): CoreText (GPUI's rasteriser settings, written by
    // ab-gpui --app text), deka today (fontdue), parley+vello hinted, unhinted.
    let dir = common::shots_dir();
    for scale in [1, 2] {
        let names = ["coretext", "deka-fontdue", "parley-vello-hinted", "parley-vello-unhinted"];
        let inputs: Vec<_> = names.iter().map(|n| dir.join(format!("text-{n}-{scale}x.png"))).collect();
        if common::stack_pngs(&inputs, &dir.join(format!("text-compare-{scale}x.png"))) {
            println!("[vello] wrote text-compare-{scale}x.png");
        }
    }
    // 11-14 px lines at 1x, 3x nearest-neighbour zoom.
    let names = ["coretext", "deka-fontdue", "parley-vello-hinted", "parley-vello-unhinted"];
    let mut zooms = vec![];
    for n in names {
        let z = dir.join(format!("zoom-{n}.png"));
        if common::crop_zoom(&dir.join(format!("text-{n}-1x.png")), (0, 0, 330, 80), 3, &z) {
            zooms.push(z);
        }
    }
    if common::stack_pngs(&zooms, &dir.join("text-compare-1x-zoom3.png")) {
        println!("[vello] wrote text-compare-1x-zoom3.png");
    }
}

// ---------------------------------------------------------------------------
// Editor (parley PlainEditor)

struct EditorState {
    font_cx: FontContext,
    layout_cx: LayoutContext<Rgba>,
    editor: PlainEditor<Rgba>,
    undo: Vec<(String, (usize, usize))>,
    redo: Vec<(String, (usize, usize))>,
    clipboard: Option<arboard::Clipboard>,
    modifiers: ModifiersState,
    dragging: bool,
    cursor_pos: (f32, f32),
    origin: (f32, f32),
}

impl EditorState {
    fn new(scale: f32) -> Self {
        let mut font_cx = FontContext::new();
        let family = register_font(&mut font_cx);
        let mut editor = PlainEditor::new(20.);
        editor.set_scale(scale);
        editor.set_width(Some((WINDOW_W - 80.) * scale));
        let styles = editor.edit_styles();
        styles.insert(StyleProperty::FontFamily(parley::FontFamily::Single(parley::FontFamilyName::Named(family.clone().into()))));
        styles.insert(StyleProperty::Brush(Rgba([0x11, 0x11, 0x11, 0xff])));
        editor.set_text("Type here. Selection, clipboard, undo and IME go through parley's PlainEditor.");
        let clipboard = match arboard::Clipboard::new() {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("clipboard unavailable: {e}");
                None
            }
        };
        Self {
            font_cx,
            layout_cx: LayoutContext::new(),
            editor,
            undo: vec![],
            redo: vec![],
            clipboard,
            modifiers: ModifiersState::empty(),
            dragging: false,
            cursor_pos: (0., 0.),
            origin: (40. * scale, 60. * scale),
        }
    }

    fn snapshot(&self) -> (String, (usize, usize)) {
        let r = self.editor.raw_selection().text_range();
        (self.editor.raw_text().to_owned(), (r.start, r.end))
    }

    /// parley has no undo: we keep text+selection snapshots before each edit.
    fn checkpoint(&mut self) {
        let snap = self.snapshot();
        if self.undo.last() != Some(&snap) {
            self.undo.push(snap);
            self.redo.clear();
        }
    }

    fn restore(&mut self, snap: (String, (usize, usize))) {
        self.editor.set_text(&snap.0);
        let mut d = self.editor.driver(&mut self.font_cx, &mut self.layout_cx);
        d.select_byte_range(snap.1.0, snap.1.1);
    }

    fn undo(&mut self) {
        if let Some(snap) = self.undo.pop() {
            let now = self.snapshot();
            self.redo.push(now);
            self.restore(snap);
        }
    }

    fn redo(&mut self) {
        if let Some(snap) = self.redo.pop() {
            let now = self.snapshot();
            self.undo.push(now);
            self.restore(snap);
        }
    }

    fn copy(&mut self) -> bool {
        let Some(text) = self.editor.selected_text().map(str::to_owned) else { return false };
        match self.clipboard.as_mut().map(|c| c.set_text(text)) {
            Some(Ok(())) => true,
            Some(Err(e)) => {
                eprintln!("copy: {e}");
                false
            }
            None => false,
        }
    }

    fn paste(&mut self) {
        let text = match self.clipboard.as_mut().map(|c| c.get_text()) {
            Some(Ok(t)) => t,
            Some(Err(e)) => {
                eprintln!("paste: {e}");
                return;
            }
            None => return,
        };
        self.checkpoint();
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).insert_or_replace_selection(&text);
    }

    /// Keyboard shortcuts and editing keys (what PlainEditor does not map itself).
    fn key(&mut self, key: &Key, text: Option<&str>) {
        let shift = self.modifiers.shift_key();
        let cmd = self.modifiers.super_key();
        let alt = self.modifiers.alt_key();
        if cmd && let Key::Character(c) = key {
            {
                match c.as_str() {
                    "c" => {
                        self.copy();
                    }
                    "x" => {
                        if self.copy() {
                            self.checkpoint();
                            self.editor.driver(&mut self.font_cx, &mut self.layout_cx).delete_selection();
                        }
                    }
                    "v" => self.paste(),
                    "a" => self.editor.driver(&mut self.font_cx, &mut self.layout_cx).select_all(),
                    "z" if shift => self.redo(),
                    "z" => self.undo(),
                    _ => {}
                }
                return;
            }
        }
        let editing = matches!(key, Key::Named(NamedKey::Backspace | NamedKey::Delete | NamedKey::Enter | NamedKey::Space)) || matches!(key, Key::Character(_));
        if editing {
            self.checkpoint();
        }
        let mut d = self.editor.driver(&mut self.font_cx, &mut self.layout_cx);
        match key {
            Key::Named(NamedKey::ArrowLeft) => match (shift, alt, cmd) {
                (true, true, _) => d.select_word_left(),
                (true, false, _) => d.select_left(),
                (false, true, _) => d.move_word_left(),
                _ => d.move_left(),
            },
            Key::Named(NamedKey::ArrowRight) => match (shift, alt) {
                (true, true) => d.select_word_right(),
                (true, false) => d.select_right(),
                (false, true) => d.move_word_right(),
                _ => d.move_right(),
            },
            Key::Named(NamedKey::ArrowUp) => if shift { d.select_up() } else { d.move_up() },
            Key::Named(NamedKey::ArrowDown) => if shift { d.select_down() } else { d.move_down() },
            Key::Named(NamedKey::Home) => if shift { d.select_to_line_start() } else { d.move_to_line_start() },
            Key::Named(NamedKey::End) => if shift { d.select_to_line_end() } else { d.move_to_line_end() },
            Key::Named(NamedKey::Backspace) => if alt { d.backdelete_word() } else { d.backdelete() },
            Key::Named(NamedKey::Delete) => if alt { d.delete_word() } else { d.delete() },
            Key::Named(NamedKey::Enter) => d.insert_or_replace_selection("\n"),
            Key::Named(NamedKey::Space) => d.insert_or_replace_selection(" "),
            _ => {
                if let Some(t) = text.filter(|t| !t.chars().any(char::is_control)) {
                    d.insert_or_replace_selection(t);
                }
            }
        }
    }

    fn ime(&mut self, ime: &Ime) {
        let mut d = self.editor.driver(&mut self.font_cx, &mut self.layout_cx);
        match ime {
            Ime::Preedit(text, cursor) => {
                if text.is_empty() {
                    d.clear_compose();
                } else {
                    d.set_compose(text, *cursor);
                }
            }
            Ime::Commit(text) => {
                d.finish_compose();
                d.insert_or_replace_selection(text);
            }
            Ime::Enabled | Ime::Disabled => {}
        }
    }

    fn mouse_down(&mut self, click_count: u32) {
        let (x, y) = (self.cursor_pos.0 - self.origin.0, self.cursor_pos.1 - self.origin.1);
        let shift = self.modifiers.shift_key();
        let mut d = self.editor.driver(&mut self.font_cx, &mut self.layout_cx);
        match (click_count, shift) {
            (_, true) => d.extend_selection_to_point(x, y),
            (2, _) => d.select_word_at_point(x, y),
            (3, _) => d.select_line_at_point(x, y),
            _ => d.move_to_point(x, y),
        }
        self.dragging = true;
    }

    fn mouse_moved(&mut self, x: f32, y: f32) {
        self.cursor_pos = (x, y);
        if self.dragging {
            let (lx, ly) = (x - self.origin.0, y - self.origin.1);
            self.editor.driver(&mut self.font_cx, &mut self.layout_cx).extend_selection_to_point(lx, ly);
        }
    }

    fn draw(&mut self, scene: &mut Scene, width: f64, height: f64) {
        scene.reset();
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None, &Rect::new(0., 0., width, height));
        let o = (self.origin.0 as f64, self.origin.1 as f64);
        let at = Affine::translate(o);
        let selection_color = Color::from_rgba8(0x9c, 0xc3, 0xff, 0xff);
        for (bb, _) in self.editor.selection_geometry() {
            scene.fill(Fill::NonZero, at, selection_color, None, &Rect::new(bb.x0, bb.y0, bb.x1, bb.y1));
        }
        let layout = self.editor.layout(&mut self.font_cx, &mut self.layout_cx).clone();
        draw_layout(scene, &layout, o, true);
        if self.editor.raw_compose().is_some() {
            // While composing, ime_cursor_area covers the preedit text: underline it.
            let a = self.editor.ime_cursor_area();
            scene.fill(Fill::NonZero, at, Color::BLACK, None, &Rect::new(a.x0, a.y1 - 2., a.x1, a.y1));
        }
        if let Some(bb) = self.editor.cursor_geometry(2.) {
            scene.fill(Fill::NonZero, at, Color::BLACK, None, &Rect::new(bb.x0, bb.y0, bb.x1, bb.y1));
        }
        let ime = self.editor.ime_cursor_area();
        scene.stroke(&Stroke::new(1.), at, Color::from_rgba8(0, 0, 0, 0x30), None, &Rect::new(ime.x0, ime.y0, ime.x1, ime.y1));
    }

    /// Scripted test of what PlainEditor + our glue provide. Returns a log.
    fn script(&mut self) -> Vec<String> {
        let mut log = vec![];
        let check = |log: &mut Vec<String>, name: &str, ok: bool, detail: String| {
            log.push(format!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" }));
        };
        self.editor.set_text("");
        for c in "Hello wörld 👋".chars() {
            let s = c.to_string();
            self.key(&Key::Character(s.as_str().into()), Some(&s));
        }
        let t = self.editor.raw_text().to_owned();
        check(&mut log, "typing (incl. ö and emoji)", t == "Hello wörld 👋", format!("{t:?}"));
        self.modifiers = ModifiersState::SHIFT;
        for _ in 0..7 {
            self.key(&Key::Named(NamedKey::ArrowLeft), None);
        }
        self.modifiers = ModifiersState::empty();
        let sel = self.editor.selected_text().map(str::to_owned);
        check(&mut log, "shift+left x7 selects by grapheme", sel.as_deref() == Some("wörld 👋"), format!("{sel:?}"));
        self.modifiers = ModifiersState::SUPER;
        self.key(&Key::Character("c".into()), None);
        let clip = self.clipboard.as_mut().map(|c| c.get_text());
        let clip_ok = matches!(&clip, Some(Ok(s)) if s == "wörld 👋");
        check(&mut log, "cmd+c copies via arboard", clip_ok, format!("{clip:?}"));
        self.modifiers = ModifiersState::empty();
        self.key(&Key::Named(NamedKey::End), None);
        self.key(&Key::Named(NamedKey::Space), None);
        self.modifiers = ModifiersState::SUPER;
        self.key(&Key::Character("v".into()), None);
        self.modifiers = ModifiersState::empty();
        let t = self.editor.raw_text().to_owned();
        check(&mut log, "cmd+v pastes", t == "Hello wörld 👋 wörld 👋", format!("{t:?}"));
        self.modifiers = ModifiersState::SUPER;
        self.key(&Key::Character("z".into()), None);
        let t1 = self.editor.raw_text().to_owned();
        self.key(&Key::Character("z".into()), None);
        let t2 = self.editor.raw_text().to_owned();
        self.modifiers = ModifiersState::SUPER | ModifiersState::SHIFT;
        self.key(&Key::Character("z".into()), None);
        let t3 = self.editor.raw_text().to_owned();
        self.modifiers = ModifiersState::empty();
        check(&mut log, "undo (ours) x2, redo (ours)", t1 == "Hello wörld 👋 " && t2 == "Hello wörld 👋" && t3 == "Hello wörld 👋 ",
            format!("{t1:?} / {t2:?} / {t3:?}"));
        // IME: the same calls winit's Ime events make.
        self.key(&Key::Named(NamedKey::End), None);
        self.ime(&Ime::Preedit("にほんご".into(), Some((0, 12))));
        let composing = self.editor.is_composing();
        let pre = self.editor.raw_text().to_owned();
        self.ime(&Ime::Preedit(String::new(), None));
        self.ime(&Ime::Commit("日本語".into()));
        let t = self.editor.raw_text().to_owned();
        check(&mut log, "IME preedit shown then commit", composing && pre.ends_with("にほんご") && t.ends_with(" 日本語"),
            format!("preedit buffer {pre:?} -> {t:?}"));
        let area = self.editor.ime_cursor_area();
        check(&mut log, "ime_cursor_area for set_ime_cursor_area", area.x1 > area.x0, format!("{area:?}"));
        log
    }
}

// ---------------------------------------------------------------------------
// The winit app

enum Content {
    World(World),
    Ui { host: Host<common::UiApp>, renderer: deka_native_ui::scene::Renderer, scene: DekaScene },
    Canvas { graph: graph::Graph, run: usize },
    Editor(Box<EditorState>),
}

struct VelloApp {
    args: Args,
    protocol: Protocol,
    started: Instant,
    times: StartupTimes,
    window: Option<Arc<Window>>,
    instance: wgpu::Instance,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    gpu: Option<Gpu>,
    target: Option<Target>,
    blitter: Option<wgpu::util::TextureBlitter>,
    content: Option<Content>,
    scene: Scene,
    deka: DekaVello,
    clock: Instant,
    idle_deadline: Option<Instant>,
    quit: bool,
    error: Option<String>,
    click: (Instant, u32),
    cursor: (f64, f64),
    occluded: bool,
}

impl VelloApp {
    fn scale(&self) -> f64 {
        self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(2.)
    }

    fn reconfigure(&mut self, size: PhysicalSize<u32>) {
        let (Some(surface), Some(config), Some(gpu)) = (&self.surface, self.config.as_mut(), &self.gpu) else { return };
        if size.width == 0 || size.height == 0 {
            return;
        }
        config.width = size.width;
        config.height = size.height;
        surface.configure(&gpu.device, config);
        self.target = Some(new_target(&gpu.device, size.width, size.height));
    }

    fn teardown(&mut self, event_loop: &ActiveEventLoop) {
        let t = Instant::now();
        if let (Some(gpu), Some(target), false) = (&self.gpu, &self.target, self.args.first_frame || self.args.interactive) {
            let name = match self.content {
                Some(Content::World(_)) => "world-vello.png",
                Some(Content::Ui { .. }) => "ui-vello.png",
                Some(Content::Canvas { .. }) => "canvas-10k-vello.png",
                _ => "",
            };
            if !name.is_empty() && !self.args.resize {
                common::write_png(&common::shots_dir().join(name), target.width, target.height, &read_rgba(gpu, target));
            }
        }
        if let Some(gpu) = &self.gpu {
            println!("[vello] GPU memory (MTLDevice.currentAllocatedSize) before teardown: {:.1} MB", common::gpu_mb());
            if let Err(e) = gpu.device.poll(wgpu::PollType::wait_indefinitely()) {
                eprintln!("poll: {e}");
            }
        }
        self.content = None;
        self.target = None;
        self.blitter = None;
        self.surface = None;
        self.gpu = None;
        println!("[vello] teardown {:.2} ms", t.elapsed().as_secs_f64() * 1000.);
        event_loop.exit();
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            self.teardown(event_loop);
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
        let Some(target) = &self.target else { return };
        let (tw, th) = (target.width, target.height);
        let (lw, lh) = (tw as f64 / scale, th as f64 / scale);
        let ms = self.clock.elapsed().as_secs_f64() * 1000.;
        let mut animating = true;
        let mut base = Color::WHITE;
        match self.content.as_mut() {
            Some(Content::World(world)) => {
                let s = world.frame(lw as f32, lh as f32, ms, false);
                animating = s.animating;
                self.deka.encode(&s, scale, &mut self.scene);
            }
            Some(Content::Ui { host, renderer, scene }) => {
                *scene = renderer.render_at(&host.render(), lw as f32, lh as f32, scale as f32, ms, false);
                animating = scene.animating;
                self.deka.encode(scene, scale, &mut self.scene);
            }
            Some(Content::Canvas { graph, .. }) => {
                graph.update(ms / 1000.);
                graph.encode(&mut self.scene, scale);
                base = graph::BACKGROUND;
            }
            Some(Content::Editor(ed)) => {
                ed.draw(&mut self.scene, tw as f64, th as f64);
                animating = false;
                if let Some(w) = &self.window {
                    let a = ed.editor.ime_cursor_area();
                    let (ox, oy) = (ed.origin.0 as f64, ed.origin.1 as f64);
                    w.set_ime_cursor_area(
                        winit::dpi::PhysicalPosition::new(a.x0 + ox, a.y0 + oy),
                        PhysicalSize::new((a.x1 - a.x0).max(1.), (a.y1 - a.y0).max(1.)),
                    );
                }
            }
            None => return,
        }
        let (Some(gpu), Some(surface), Some(blitter), Some(target)) = (self.gpu.as_mut(), &self.surface, &self.blitter, &self.target) else { return };
        if let Err(e) = render(gpu, &self.scene, target, base) {
            self.error = Some(e);
            self.quit = true;
            return;
        }
        let work = t0.elapsed();
        let tp = Instant::now();
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Occluded => {
                // Not visible (window not shown yet, display asleep, covered):
                // draw nothing and wait for WindowEvent::Occluded(false).
                if !self.occluded {
                    eprintln!("[vello] occluded: pausing until visible");
                }
                self.occluded = true;
                return;
            }
            other => {
                // Outdated / lost / timeout: reconfigure and try once more.
                eprintln!("[vello] skipped frame ({other:?})");
                if let Some(w) = self.window.clone() {
                    self.reconfigure(w.inner_size());
                    w.request_redraw();
                }
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        blitter.copy(&gpu.device, &mut enc, &target.view, &view);
        gpu.queue.submit([enc.finish()]);
        if let Some(w) = &self.window {
            w.pre_present_notify();
        }
        gpu.queue.present(frame);
        if self.protocol.measuring() {
            self.protocol.present.push(tp.elapsed());
        }
        let first = !self.protocol.first_frame_done();
        let step = self.protocol.end_frame(work);
        if first {
            println!(
                "[vello] startup: window {:.1} ms, adapter+device {:.1} ms, vello Renderer::new (shader compile) {:.1} ms, first frame work {:.1} ms; GPU after device {:.1} MB, after vello renderer {:.1} MB, after first frame {:.1} MB",
                self.times.window_ms,
                self.times.adapter_device_ms,
                self.times.vello_renderer_ms,
                work.as_secs_f64() * 1000.,
                self.times.gpu_after_device_mb,
                self.times.gpu_after_renderer_mb,
                common::gpu_mb()
            );
            if let Some(secs) = self.args.idle_secs {
                self.idle_deadline = Some(Instant::now() + Duration::from_secs_f64(secs));
            }
        }
        if let Step::Quit = step {
            if let Some(Content::Canvas { run, .. }) = self.content.as_mut()
                && *run + 1 < self.args.shapes.len()
            {
                // Next canvas scene size.
                *run += 1;
                let shapes = self.args.shapes[*run];
                let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
                self.content = Some(Content::Canvas { graph: graph::Graph::new(shapes, lw, lh, layout), run: *run });
                self.protocol = Protocol::new(self.args.clone(), "vello");
                println!("[vello] canvas shapes {shapes} ({layout:?})");
            } else {
                self.quit = true;
            }
        }
        let keep_animating = self.quit
            || (self.args.idle_secs.is_none() && !matches!(self.content, Some(Content::Editor(_))))
            || (animating && self.args.idle_secs.is_none())
            || (self.args.first_frame);
        if keep_animating && let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

impl ApplicationHandler for VelloApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        common::trace("resumed");
        let t = Instant::now();
        let mut attrs = Window::default_attributes()
            .with_title("deka A/B: winit + wgpu + vello")
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
        common::trace("window created");
        let surface = match self.instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("surface: {e}");
                event_loop.exit();
                return;
            }
        };
        let gpu = match new_gpu(&self.instance, Some(&surface), &mut self.times) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("{e}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&gpu.device, &config);
        common::trace("device + vello renderer ready, surface configured");
        self.blitter = Some(wgpu::util::TextureBlitter::new(&gpu.device, format));
        self.target = Some(new_target(&gpu.device, size.width, size.height));
        let scale = window.scale_factor();
        let (lw, lh) = (size.width as f64 / scale, size.height as f64 / scale);
        self.content = Some(match self.args.app {
            AppKind::Ui => Content::Ui {
                host: Host::new(common::UiApp),
                renderer: deka_native_ui::scene::Renderer::new(),
                scene: DekaScene::default(),
            },
            AppKind::Canvas => {
                let shapes = self.args.shapes.first().copied().unwrap_or(1_000);
                let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
                println!("[vello] canvas shapes {shapes} ({layout:?})");
                Content::Canvas { graph: graph::Graph::new(shapes, lw, lh, layout), run: 0 }
            }
            AppKind::Editor => {
                window.set_ime_allowed(true);
                let mut ed = EditorState::new(scale as f32);
                if !self.args.interactive {
                    for line in ed.script() {
                        println!("[vello] editor {line}");
                    }
                }
                Content::Editor(Box::new(ed))
            }
            _ => {
                let mut world = World::new();
                world.start();
                world.set_muted(true);
                Content::World(world)
            }
        });
        self.surface = Some(surface);
        self.config = Some(config);
        self.gpu = Some(gpu);
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
                self.reconfigure(size);
                // Draw the new size before the compositor shows it (no stretching).
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if !self.protocol.first_frame_done() {
                    common::trace("RedrawRequested");
                }
                self.frame(event_loop);
                if let Some(e) = self.error.take() {
                    eprintln!("[vello] {e}");
                }
                // The scripted editor run shows its result for a few frames, then quits.
                if !self.args.interactive
                    && matches!(self.content, Some(Content::Editor(_)))
                    && self.protocol.frame_index() >= 3
                {
                    if let (Some(gpu), Some(target)) = (&self.gpu, &self.target) {
                        common::write_png(&common::shots_dir().join("editor.png"), target.width, target.height, &read_rgba(gpu, target));
                    }
                    self.quit = true;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                } else if !self.args.interactive
                    && matches!(self.content, Some(Content::Editor(_)))
                    && let Some(w) = &self.window
                {
                    w.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                if let Some(Content::Editor(ed)) = self.content.as_mut() {
                    ed.modifiers = m.state();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                match self.content.as_mut() {
                    Some(Content::World(world)) => {
                        let name = match &event.logical_key {
                            Key::Named(NamedKey::ArrowUp) => "up".to_owned(),
                            Key::Named(NamedKey::ArrowDown) => "down".to_owned(),
                            Key::Named(NamedKey::ArrowLeft) => "left".to_owned(),
                            Key::Named(NamedKey::ArrowRight) => "right".to_owned(),
                            Key::Named(NamedKey::Enter) => "enter".to_owned(),
                            Key::Named(NamedKey::Space) => "space".to_owned(),
                            Key::Named(NamedKey::Escape) => "escape".to_owned(),
                            Key::Character(c) => c.to_string(),
                            _ => String::new(),
                        };
                        world.key(&name, down);
                    }
                    Some(Content::Editor(ed)) if down => {
                        ed.key(&event.logical_key, event.text.as_deref());
                    }
                    _ => {}
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::Ime(ime) => {
                if let Some(Content::Editor(ed)) = self.content.as_mut() {
                    ed.ime(&ime);
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                if let Some(Content::Editor(ed)) = self.content.as_mut() {
                    ed.mouse_moved(position.x as f32, position.y as f32);
                    if ed.dragging
                        && let Some(w) = &self.window
                    {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let scale = self.scale();
                match self.content.as_mut() {
                    Some(Content::Editor(ed)) => {
                        if state == ElementState::Pressed {
                            let now = Instant::now();
                            let count = if now - self.click.0 < Duration::from_millis(400) { self.click.1 % 3 + 1 } else { 1 };
                            self.click = (now, count);
                            ed.mouse_down(count);
                        } else {
                            ed.dragging = false;
                        }
                    }
                    Some(Content::Ui { host, scene, .. }) if state == ElementState::Pressed => {
                        // deka's own hit testing on the last scene (logical px).
                        let (x, y) = ((self.cursor.0 / scale) as f32, (self.cursor.1 / scale) as f32);
                        if let Some(handler) = scene.hit(x, y).map(|t| t.handler) {
                            host.click(handler);
                        }
                    }
                    _ => {}
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
                if self.gpu.is_some() {
                    println!("[vello] GPU memory after idle: {:.1} MB", common::gpu_mb());
                }
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

/// How much of each fixed vello buffer the deka scenes need at 3200x2000.
#[cfg(feature = "diag")]
fn bump_report() {
    let mut times = StartupTimes::default();
    let instance = new_instance();
    let Ok(mut gpu) = new_gpu(&instance, None, &mut times) else { return };
    let (w, h, scale) = (3200u32, 2000u32, 2.0f64);
    let target = new_target(&gpu.device, w, h);
    let (lw, lh) = (w as f32 / scale as f32, h as f32 / scale as f32);
    let mut world = World::new();
    world.start();
    let ui_scene = deka_native_ui::scene::Renderer::new().render_at(&Host::new(common::UiApp).render(), lw, lh, scale as f32, 0., false);
    let mut scenes: Vec<(String, Scene, Color)> = vec![];
    let mut s = Scene::new();
    DekaVello::default().encode(&ui_scene, scale, &mut s);
    scenes.push(("ui".into(), s, Color::WHITE));
    let mut s = Scene::new();
    DekaVello::default().encode(&world.frame(lw, lh, 0., false), scale, &mut s);
    scenes.push(("world".into(), s, Color::WHITE));
    for shapes in [1_000usize, 10_000] {
        let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
        let mut g = graph::Graph::new(shapes, lw as f64, lh as f64, layout);
        g.update(0.);
        let mut s = Scene::new();
        g.encode(&mut s, scale);
        scenes.push((format!("canvas {shapes}"), s, graph::BACKGROUND));
    }
    for (name, scene, base) in &scenes {
        let device = gpu.device.clone();
        #[allow(deprecated, reason = "the only API that returns vello's bump counters")]
        let bump = vello::util::block_on_wgpu(&device, gpu.renderer.render_to_texture_async(
            &gpu.device, &gpu.queue, scene, &target.view,
            &RenderParams { base_color: *base, width: w, height: h, antialiasing_method: AaConfig::Area },
            vello::low_level::DebugLayers::none(),
        ));
        match bump {
            Ok(Some(b)) => println!(
                "[bump] {name:10} failed=0x{:x} tiles {} / {} | segments {} / {} | lines {} / {} | seg_counts {} | ptcl {} / {} | binning {} / {} | blend {}",
                b.failed, b.tile, 1u32 << 21, b.segments, 1u32 << 21, b.lines, 1u32 << 21, b.seg_counts, b.ptcl, 1u32 << 23, b.binning, 1u32 << 18, b.blend
            ),
            other => println!("[bump] {name}: {:?}", other.map(|b| b.is_some())),
        }
    }
}

/// GPU memory of vello itself rendering the deka UI and world scenes at
/// 3200x2000 into a texture (no window, no swapchain).
fn mem_report() {
    let mut times = StartupTimes::default();
    let instance = new_instance();
    let Ok(mut gpu) = new_gpu(&instance, None, &mut times) else { return };
    let after_device = common::gpu_mb();
    let (w, h, scale) = (3200u32, 2000u32, 2.0f64);
    let target = new_target(&gpu.device, w, h);
    let after_target = common::gpu_mb();
    let (lw, lh) = (w as f32 / scale as f32, h as f32 / scale as f32);
    let ui = deka_native_ui::scene::Renderer::new().render_at(&Host::new(common::UiApp).render(), lw, lh, scale as f32, 0., false);
    let mut world = World::new();
    world.start();
    let mut deka = DekaVello::default();
    for (name, scene) in [("ui", ui), ("world", world.frame(lw, lh, 0., false))] {
        let mut s = Scene::new();
        deka.encode(&scene, scale, &mut s);
        for _ in 0..10 {
            if let Err(e) = render(&mut gpu, &s, &target, Color::WHITE) {
                eprintln!("{e}");
            }
        }
        let rgba = read_rgba(&gpu, &target);
        let drawn = rgba.chunks_exact(4).any(|p| p[3] != 0);
        let u = common::usage();
        println!(
            "[mem] {name}: GPU {:.1} MB (device {:.1}, + 3200x2000 target {:.1}); footprint {:.1} MB; frame actually drawn: {drawn}",
            common::gpu_mb(),
            after_device,
            after_target - after_device,
            common::mb(u.footprint)
        );
        common::write_png(&common::shots_dir().join(format!("mem-{name}.png")), w, h, &rgba);
    }
}

/// Compute vello on a device limited to WebGL2 capabilities (expected to fail).
fn webgl2_limits() {
    let instance = new_instance();
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let (device, _queue) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
        ..Default::default()
    })) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let errors = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    {
        let errors = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e| {
            if let Ok(mut v) = errors.lock() {
                v.push(e.to_string());
            }
        }));
    }
    let r = Renderer::new(&device, RendererOptions { antialiasing_support: AaSupport::area_only(), ..Default::default() });
    if let Err(e) = device.poll(wgpu::PollType::wait_indefinitely()) {
        eprintln!("poll: {e}");
    }
    let errs = errors.lock().map(|v| v.clone()).unwrap_or_default();
    println!("[vello webgl2-limits] Renderer::new -> {}; wgpu validation errors: {}", if r.is_ok() { "Ok" } else { "Err" }, errs.len());
    if let Err(e) = r {
        println!("  {e}");
    }
    for e in errs.iter().take(2) {
        println!("  {}", e.lines().take(3).collect::<Vec<_>>().join(" | "));
    }
}

pub fn run(args: Args) {
    if args.app == AppKind::WebGl2 {
        webgl2_limits();
        return;
    }
    if args.app == AppKind::Mem {
        mem_report();
        return;
    }
    if args.app == AppKind::Bump {
        #[cfg(feature = "diag")]
        bump_report();
        #[cfg(not(feature = "diag"))]
        eprintln!("--app bump needs --features diag");
        return;
    }
    if args.app == AppKind::Text {
        text_sheets();
        return;
    }
    common::trace("main");
    let started = Instant::now();
    let event_loop = match EventLoop::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("event loop: {e}");
            return;
        }
    };
    let mut app = VelloApp {
        protocol: Protocol::new(args.clone(), "vello"),
        args,
        started,
        times: StartupTimes::default(),
        window: None,
        instance: new_instance(),
        surface: None,
        config: None,
        gpu: None,
        target: None,
        blitter: None,
        content: None,
        scene: Scene::new(),
        deka: DekaVello::default(),
        clock: Instant::now(),
        idle_deadline: None,
        quit: false,
        error: None,
        click: (Instant::now() - Duration::from_secs(1), 0),
        cursor: (0., 0.),
        occluded: false,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop: {e}");
    }
    println!("[vello] process lifetime in event loop {:.0} ms", app.started.elapsed().as_secs_f64() * 1000.);
}
