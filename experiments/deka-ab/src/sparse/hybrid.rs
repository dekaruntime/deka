//! vello_gpu (formerly vello_hybrid): CPU builds sparse strips, the GPU
//! rasterises them with ordinary vertex/fragment shaders, straight into the
//! wgpu surface (no intermediate texture, no blit).

use super::*;
use std::sync::Mutex;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub struct HybridPresenter {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    depth: wgpu::TextureView,
    /// vello_gpu 0.3 made the depth buffer optional (`--no-depth`).
    use_depth: bool,
    scene: vello_gpu::Scene,
    images: HashMap<String, (vello_gpu::ImageId, ImageSource)>,
}

fn new_device(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, limits: wgpu::Limits) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), String> {
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: surface,
        ..Default::default()
    }))
    .map_err(|e| format!("adapter: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("deka-ab hybrid"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        ..Default::default()
    }))
    .map_err(|e| format!("device: {e}"))?;
    Ok((adapter, device, queue))
}

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

fn pixmap_of(g: &GlyphImage) -> Option<Arc<vello_cpu_pixmap::Pixmap>> {
    match pixmap_source(g) {
        ImageSource::Pixmap(p) => Some(p),
        _ => None,
    }
}
mod vello_cpu_pixmap {
    pub use vello_gpu::Pixmap;
}

impl HybridPresenter {
    fn render_to(&mut self, view: &wgpu::TextureView, depth: &wgpu::TextureView, w: u32, h: u32) -> Result<(), String> {
        self.scene.flush();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer
            .render(
                &self.scene,
                &mut self.resources,
                &self.device,
                &self.queue,
                &mut encoder,
                &vello_gpu::RenderSize { width: w as u16, height: h as u16 },
                view,
                self.use_depth.then_some(depth),
                &vello_gpu::TextureBindings::new(),
                vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()),
            )
            .map_err(|e| format!("vello_gpu render: {e:?}"))?;
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

impl Presenter for HybridPresenter {
    const NAME: &'static str = "vello_gpu";

    fn new(window: Arc<Window>, times: &mut StartupTimes) -> Result<Self, String> {
        let t = Instant::now();
        let instance = instance();
        let surface = instance.create_surface(window.clone()).map_err(|e| format!("surface: {e}"))?;
        let (_, device, queue) = new_device(&instance, Some(&surface), wgpu::Limits::default())?;
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: FORMAT,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        times.device_ms = t.elapsed().as_secs_f64() * 1000.;
        let t = Instant::now();
        let mut settings = vello_gpu::RenderSettings::default();
        if std::env::args().any(|a| a == "--small-atlas") {
            // Default image atlas is 4096x4096 RGBA (64 MB) on first upload; deka's
            // glyph bitmaps fit in 1024x1024 (4 MB), which still grows on demand.
            settings.memory_settings.image_atlas_config.atlas_size = (1024, 1024);
            println!("[vello_gpu] image atlas 1024x1024 (auto-grow)");
        }
        let (mut renderer, resources) = vello_gpu::Renderer::new_with(
            &device,
            &vello_gpu::RenderTargetConfig { format: FORMAT, width: size.width as u16, height: size.height as u16 },
            settings,
        );
        // Fork: `--no-belt` restores the stock per-call staging uploads.
        renderer.set_staging_belt(!std::env::args().any(|a| a == "--no-belt"));
        let no_depth = std::env::args().any(|a| a == "--no-depth");
        // Without depth testing, keep only a 1x1 placeholder (the real one is ~25 MB at 3200x2000).
        let depth_size = if no_depth { vello_gpu::RenderSize { width: 1, height: 1 } } else { vello_gpu::RenderSize { width: size.width as u16, height: size.height as u16 } };
        let depth = vello_gpu::Renderer::create_depth_texture_view(&device, &depth_size);
        times.renderer_ms = t.elapsed().as_secs_f64() * 1000.;
        Ok(Self {
            scene: new_scene(size.width as u16, size.height as u16),
            window,
            surface,
            config,
            device,
            queue,
            renderer,
            resources,
            depth,
            use_depth: !std::env::args().any(|a| a == "--no-depth"),
            images: HashMap::new(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 || (size.width, size.height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        let rs = vello_gpu::RenderSize { width: size.width as u16, height: size.height as u16 };
        if self.use_depth {
            self.depth = vello_gpu::Renderer::create_depth_texture_view(&self.device, &rs);
        }
        self.scene = new_scene(rs.width, rs.height);
    }

    fn encode_deka(&mut self, deka: &DekaScene, scale: f64) {
        self.scene.reset();
        let used = used_ids(deka);
        let missing: Vec<&GlyphImage> = deka.images.iter().filter(|g| used.contains(g.id.as_str()) && !self.images.contains_key(&g.id)).collect();
        let stale: Vec<String> = self.images.keys().filter(|id| !used.contains(id.as_str())).cloned().collect();
        if !missing.is_empty() || !stale.is_empty() {
            // vello_gpu panics on pixmap image sources: upload to its atlas, draw by id.
            let mut encoder = self.device.create_command_encoder(&Default::default());
            for id in stale {
                if let Some((image_id, _)) = self.images.remove(&id) {
                    self.renderer.destroy_image(&mut self.resources, &mut encoder, image_id);
                }
            }
            for g in missing {
                if let Some(pixmap) = pixmap_of(g) {
                    let image_id = self.renderer.upload_image(&mut self.resources, &self.device, &self.queue, &mut encoder, &pixmap);
                    self.images.insert(g.id.clone(), (image_id, ImageSource::opaque_id_with_transparency_hint(image_id, true)));
                }
            }
            self.queue.submit([encoder.finish()]);
        }
        let sources: HashMap<String, ImageSource> = self.images.iter().map(|(k, (_, s))| (k.clone(), s.clone())).collect();
        encode_deka(&mut self.scene, deka, scale, &sources);
    }

    fn encode_graph(&mut self, graph: &graph::Graph, scale: f64) {
        self.scene.reset();
        encode_graph(&mut self.scene, graph, scale, self.config.width as f64, self.config.height as f64);
    }

    fn present(&mut self) -> Result<Presented, String> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(Presented::Occluded),
            _ => {
                self.surface.configure(&self.device, &self.config);
                return Ok(Presented::Retry);
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let depth = self.depth.clone();
        let (w, h) = (self.config.width, self.config.height);
        self.render_to(&view, &depth, w, h)?;
        self.window.pre_present_notify();
        self.queue.present(frame);
        Ok(Presented::Done)
    }

    fn screenshot(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let (w, h) = (self.config.width, self.config.height);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let depth = self.depth.clone();
        self.render_to(&view, &depth, w, h).ok()?;
        let mut rgba = readback(&self.device, &self.queue, &texture, w, h);
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        Some((w, h, rgba))
    }
}

/// Scene with the fork's settings from the command line:
/// `--threads N` (parallel strips) and `--strip-cache`.
pub fn new_scene(w: u16, h: u16) -> vello_gpu::Scene {
    let mut scene = vello_gpu::Scene::new(w, h);
    let args: Vec<String> = std::env::args().collect();
    let threads = args.iter().position(|a| a == "--threads").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(1);
    scene.set_parallelism(threads);
    scene.set_strip_cache(args.iter().any(|a| a == "--strip-cache"));
    scene
}

fn readback(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
    let padded = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    if let Err(e) = device.poll(wgpu::PollType::wait_indefinitely()) {
        eprintln!("poll: {e}");
    }
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    if let Ok(data) = buffer.slice(..).get_mapped_range() {
        for row in data.chunks_exact(padded as usize) {
            out.extend_from_slice(&row[..(w * 4) as usize]);
        }
    }
    out
}

/// Offscreen: the coverage sheet, the deka UI and world scenes and both canvas
/// scenes, optionally on a device limited to what WebGL2 offers (no compute,
/// no storage buffers). wgpu validation errors are collected, not panicked on.
pub fn offscreen(webgl2_limits: bool) {
    let label = if webgl2_limits { "webgl2-limits" } else { "default-limits" };
    let instance = instance();
    let limits = if webgl2_limits { wgpu::Limits::downlevel_webgl2_defaults() } else { wgpu::Limits::default() };
    let (adapter, device, queue) = match new_device(&instance, None, limits) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[vello_gpu] {e}");
            return;
        }
    };
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let errors = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e| {
            if let Ok(mut v) = errors.lock() {
                v.push(e.to_string());
            }
        }));
    }
    let l = device.limits();
    println!(
        "[vello_gpu {label}] adapter {}; device limits: compute invocations {}, storage buffers/stage {}, storage textures/stage {}",
        adapter.get_info().name,
        l.max_compute_invocations_per_workgroup,
        l.max_storage_buffers_per_shader_stage,
        l.max_storage_textures_per_shader_stage
    );
    let (w, h) = COVERAGE;
    let rs = vello_gpu::RenderSize { width: w as u16, height: h as u16 };
    let t = Instant::now();
    let (mut renderer, mut resources) = vello_gpu::Renderer::new(&device, &vello_gpu::RenderTargetConfig { format: wgpu::TextureFormat::Rgba8Unorm, width: w as u16, height: h as u16 });
    println!("[vello_gpu {label}] Renderer::new {:.1} ms", t.elapsed().as_secs_f64() * 1000.);
    let depth = vello_gpu::Renderer::create_depth_texture_view(&device, &rs);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let render = |scene: &mut vello_gpu::Scene, renderer: &mut vello_gpu::Renderer, resources: &mut vello_gpu::Resources| {
        scene.flush();
        let mut encoder = device.create_command_encoder(&Default::default());
        let r = renderer.render(scene, resources, &device, &queue, &mut encoder, &rs, &view, Some(&depth), &vello_gpu::TextureBindings::new(), vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()));
        queue.submit([encoder.finish()]);
        r.map_err(|e| format!("{e:?}"))
    };
    // Coverage image goes through the atlas (pixmap sources panic on vello_gpu).
    let image = {
        let mut encoder = device.create_command_encoder(&Default::default());
        let id = pixmap_of(&coverage_image()).map(|p| renderer.upload_image(&mut resources, &device, &queue, &mut encoder, &p));
        queue.submit([encoder.finish()]);
        id.map(|id| ImageSource::opaque_id_with_transparency_hint(id, false))
    };
    let Some(image) = image else { return };
    // Probe each blend mode in isolation.
    let mut failing = vec![];
    for (name, mix, compose) in BLEND_MODES {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut scene = new_scene(w as u16, h as u16);
            let cell = Rect::new(0., 0., 64., 64.);
            scene.push_layer(Some(&cell.to_path(TOLERANCE)), None, None, None, None);
            scene.set_paint(Color::BLACK);
            scene.fill_rect(&Rect::new(0., 0., 32., 32.));
            scene.push_layer(Some(&cell.to_path(TOLERANCE)), Some(BlendMode::new(mix, compose)), None, None, None);
            scene.set_paint(Color::WHITE);
            scene.fill_rect(&Rect::new(16., 16., 48., 48.));
            scene.pop_layer();
            scene.pop_layer();
            render(&mut scene, &mut renderer, &mut resources)
        }));
        if !matches!(ok, Ok(Ok(()))) {
            failing.push(name);
        }
    }
    println!("[vello_gpu {label}] blend modes that panic or error: {failing:?}");
    let mut scene = new_scene(w as u16, h as u16);
    coverage(&mut scene, &mut resources, &image, &failing);
    match render(&mut scene, &mut renderer, &mut resources) {
        Ok(()) => {
            let rgba = readback(&device, &queue, &texture, w, h);
            common::write_png(&common::shots_dir().join(format!("coverage-vello_gpu-{label}.png")), w, h, &rgba);
            println!("[vello_gpu {label}] wrote coverage-vello_gpu-{label}.png");
        }
        Err(e) => println!("[vello_gpu {label}] coverage render failed: {e}"),
    }
    // Filter effects (drop shadow on a path, blurred text).
    let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut scene = new_scene(w as u16, h as u16);
        filter_sheet(&mut scene, &mut resources);
        render(&mut scene, &mut renderer, &mut resources)
    }));
    match ok {
        Ok(Ok(())) => {
            let rgba = readback(&device, &queue, &texture, w, h);
            let (fw, fh) = FILTER_SHEET;
            let mut crop = Vec::with_capacity((fw * fh * 4) as usize);
            for y in 0..fh {
                let s = (y * w * 4) as usize;
                crop.extend_from_slice(&rgba[s..s + (fw * 4) as usize]);
            }
            common::write_png(&common::shots_dir().join(format!("filters-vello_gpu-{label}.png")), fw, fh, &crop);
            println!("[vello_gpu {label}] filter sheet rendered");
        }
        Ok(Err(e)) => println!("[vello_gpu {label}] filter sheet error: {e}"),
        Err(_) => println!("[vello_gpu {label}] filter sheet PANICKED"),
    }
    // The canvas scenes too (heavier paths), to see they need nothing more.
    for shapes in [1_000usize, 10_000] {
        let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
        let mut g = graph::Graph::new(shapes, w as f64 / 2., h as f64 / 2., layout);
        g.update(0.);
        let mut scene = new_scene(w as u16, h as u16);
        encode_graph(&mut scene, &g, 2.0, w as f64, h as f64);
        let r = render(&mut scene, &mut renderer, &mut resources);
        println!("[vello_gpu {label}] canvas {shapes}: {:?}", r.map(|_| "rendered"));
    }
    if let Err(e) = device.poll(wgpu::PollType::wait_indefinitely()) {
        eprintln!("poll: {e}");
    }
    let errs = errors.lock().map(|v| v.clone()).unwrap_or_default();
    println!("[vello_gpu {label}] wgpu validation errors: {}", errs.len());
    for e in errs.iter().take(3) {
        println!("  {}", e.lines().take(4).collect::<Vec<_>>().join(" | "));
    }
}

/// Offscreen split of vello_gpu frame cost: scene encode (CPU strip
/// generation) vs render (schedule, uploads, GPU) for the canvas and world.
pub fn bench(frames: usize) {
    let instance = instance();
    let Ok((_, device, queue)) = new_device(&instance, None, wgpu::Limits::default()) else { return };
    let (w, h) = (3200u32, 2000u32);
    let rs = vello_gpu::RenderSize { width: w as u16, height: h as u16 };
    let (mut renderer, mut resources) = vello_gpu::Renderer::new(&device, &vello_gpu::RenderTargetConfig { format: wgpu::TextureFormat::Rgba8Unorm, width: w as u16, height: h as u16 });
    let depth = vello_gpu::Renderer::create_depth_texture_view(&device, &rs);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut scene = new_scene(w as u16, h as u16);
    for shapes in [1_000usize, 10_000] {
        let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
        let mut g = graph::Graph::new(shapes, w as f64 / 2., h as f64 / 2., layout);
        let (mut enc_t, mut render_t, mut gpu_t) = (crate::common::Series::default(), crate::common::Series::default(), crate::common::Series::default());
        for f in 0..frames + 10 {
            g.update(f as f64 / 60.);
            let t0 = Instant::now();
            scene.reset();
            encode_graph(&mut scene, &g, 2.0, w as f64, h as f64);
            let t1 = Instant::now();
            scene.flush();
            let mut encoder = device.create_command_encoder(&Default::default());
            if let Err(e) = renderer.render(&scene, &mut resources, &device, &queue, &mut encoder, &rs, &view, Some(&depth), &vello_gpu::TextureBindings::new(), vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default())) {
                eprintln!("{e:?}");
            }
            queue.submit([encoder.finish()]);
            let t2 = Instant::now();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let t3 = Instant::now();
            if f >= 10 {
                enc_t.push(t1 - t0);
                render_t.push(t2 - t1);
                gpu_t.push(t3 - t2);
            }
        }
        println!("[vello_gpu bench] canvas {shapes}: scene encode (strips) {}\n                     render() CPU {}\n                     GPU wait {}", enc_t.summary(), render_t.summary(), gpu_t.summary());
    }
    // deka's own scenes: the animating world and the static UI.
    let mut world = World::new();
    world.start();
    let ui_host = Host::new(crate::common::UiApp);
    let deka_renderer = deka_native_ui::scene::Renderer::new();
    let mut images: HashMap<String, (vello_gpu::ImageId, ImageSource)> = HashMap::new();
    for name in ["world", "ui"] {
        let (mut build_t, mut enc_t, mut render_t, mut gpu_t) = Default::default();
        let (build_t, enc_t, render_t, gpu_t): (&mut crate::common::Series, &mut crate::common::Series, &mut crate::common::Series, &mut crate::common::Series) = (&mut build_t, &mut enc_t, &mut render_t, &mut gpu_t);
        let mut paths = 0usize;
        for f in 0..frames + 10 {
            let t0 = Instant::now();
            let deka = if name == "world" {
                world.frame(1600., 1000., f as f64 * 16.7, false)
            } else {
                deka_renderer.render_at(&ui_host.render(), 1600., 1000., 2., f as f64 * 16.7, false)
            };
            let t1 = Instant::now();
            let mut encoder = device.create_command_encoder(&Default::default());
            for g in &deka.images {
                if !images.contains_key(&g.id)
                    && let Some(p) = pixmap_of(g)
                {
                    let id = renderer.upload_image(&mut resources, &device, &queue, &mut encoder, &p);
                    images.insert(g.id.clone(), (id, ImageSource::opaque_id_with_transparency_hint(id, true)));
                }
            }
            queue.submit([encoder.finish()]);
            let sources: HashMap<String, ImageSource> = images.iter().map(|(k, (_, s))| (k.clone(), s.clone())).collect();
            let t2 = Instant::now();
            scene.reset();
            encode_deka(&mut scene, &deka, 2.0, &sources);
            let t3 = Instant::now();
            paths = deka.paint.len();
            scene.flush();
            let mut encoder = device.create_command_encoder(&Default::default());
            if let Err(e) = renderer.render(&scene, &mut resources, &device, &queue, &mut encoder, &rs, &view, Some(&depth), &vello_gpu::TextureBindings::new(), vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default())) {
                eprintln!("{e:?}");
            }
            queue.submit([encoder.finish()]);
            let t4 = Instant::now();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let t5 = Instant::now();
            if f >= 10 {
                build_t.push(t1 - t0);
                enc_t.push(t3 - t2);
                render_t.push(t4 - t3);
                gpu_t.push(t5 - t4);
                let _ = t2;
            }
        }
        println!("[vello_gpu bench] {name} ({paths} paints): deka scene build {}\n                     scene encode {}\n                     render() CPU {}\n                     GPU wait {}", build_t.summary(), enc_t.summary(), render_t.summary(), gpu_t.summary());
    }
}

/// Offscreen GPU context shared by `verify` and `scaling`.
struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    depth: wgpu::TextureView,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    images: HashMap<String, ImageSource>,
    w: u32,
    h: u32,
}

impl Offscreen {
    fn new(w: u32, h: u32) -> Option<Self> {
        let instance = instance();
        let (_, device, queue) = new_device(&instance, None, wgpu::Limits::default()).ok()?;
        let rs = vello_gpu::RenderSize { width: w as u16, height: h as u16 };
        let (mut renderer, resources) = vello_gpu::Renderer::new(&device, &vello_gpu::RenderTargetConfig { format: wgpu::TextureFormat::Rgba8Unorm, width: w as u16, height: h as u16 });
        renderer.set_staging_belt(!std::env::args().any(|a| a == "--no-belt"));
        let depth = vello_gpu::Renderer::create_depth_texture_view(&device, &rs);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Some(Self { device, queue, renderer, resources, depth, texture, view, images: HashMap::new(), w, h })
    }

    fn upload(&mut self, deka: &DekaScene) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for g in &deka.images {
            if !self.images.contains_key(&g.id)
                && let Some(p) = pixmap_of(g)
            {
                let id = self.renderer.upload_image(&mut self.resources, &self.device, &self.queue, &mut encoder, &p);
                self.images.insert(g.id.clone(), ImageSource::opaque_id_with_transparency_hint(id, true));
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Flush, render, and return (render() CPU time, GPU wait time).
    fn render(&mut self, scene: &mut vello_gpu::Scene) -> (Duration, Duration) {
        scene.flush();
        let t0 = Instant::now();
        let rs = vello_gpu::RenderSize { width: self.w as u16, height: self.h as u16 };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if let Err(e) = self.renderer.render(scene, &mut self.resources, &self.device, &self.queue, &mut encoder, &rs, &self.view, Some(&self.depth), &vello_gpu::TextureBindings::new(), vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default())) {
            eprintln!("render: {e}");
        }
        self.queue.submit([encoder.finish()]);
        let t1 = Instant::now();
        if let Err(e) = self.device.poll(wgpu::PollType::wait_indefinitely()) {
            eprintln!("poll: {e}");
        }
        (t1 - t0, t1.elapsed())
    }

    fn pixels(&self) -> Vec<u8> {
        readback(&self.device, &self.queue, &self.texture, self.w, self.h)
    }
}

/// The scenes the fork must reproduce exactly.
const VERIFY_SCENES: [&str; 5] = ["coverage", "canvas-1k", "canvas-10k", "world", "ui"];

fn encode_named(off: &mut Offscreen, scene: &mut vello_gpu::Scene, name: &str, t: f64) {
    scene.reset();
    match name {
        "coverage" => {
            // Upload once: a fresh atlas slot per render changes edge filtering
            // of the rotated image (2 pixels differ even between two stock renders).
            if !off.images.contains_key("coverage") {
                let mut encoder = off.device.create_command_encoder(&Default::default());
                if let Some(p) = pixmap_of(&coverage_image()) {
                    let id = off.renderer.upload_image(&mut off.resources, &off.device, &off.queue, &mut encoder, &p);
                    off.images.insert("coverage".into(), ImageSource::opaque_id_with_transparency_hint(id, false));
                }
                off.queue.submit([encoder.finish()]);
            }
            if let Some(image) = off.images.get("coverage").cloned() {
                coverage(scene, &mut off.resources, &image, &[]);
            }
        }
        "canvas-1k" | "canvas-10k" => {
            let shapes = if name == "canvas-1k" { 1_000 } else { 10_000 };
            let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
            let mut g = graph::Graph::new(shapes, off.w as f64 / 2., off.h as f64 / 2., layout);
            // A few physics steps so the force layout is not at its start state.
            for i in 0..30 {
                g.update(t + i as f64 / 60.);
            }
            encode_graph(scene, &g, 2.0, off.w as f64, off.h as f64);
        }
        _ => {
            let deka = if name == "world" {
                let mut world = World::new();
                world.start();
                world.frame(off.w as f32 / 2., off.h as f32 / 2., t * 1000., false)
            } else {
                deka_native_ui::scene::Renderer::new().render_at(&Host::new(crate::common::UiApp).render(), off.w as f32 / 2., off.h as f32 / 2., 2., t * 1000., false)
            };
            off.upload(&deka);
            encode_deka(scene, &deka, 2.0, &off.images);
        }
    }
}

/// Pixel-compare the fork's parallel and cached strips against the stock
/// (eager, single-threaded) path for every scene. Exits non-zero on mismatch.
///
/// `--reference DIR` also compares every configuration (the first included)
/// against `DIR/verify-<scene>.png` written by an earlier build, e.g. phase 4's
/// eager path, whose renderer is upstream's.
pub fn verify() -> bool {
    let Some(mut off) = Offscreen::new(3200, 2000) else { return false };
    // (label, threads, strip cache, shape API, staging belt). The reference
    // is the stock path end to end: eager strips, BezPath API, and the stock
    // `Queue::write_*` uploads.
    let configs: [(&str, usize, bool, bool, bool); 10] = [
        ("stock (eager, uploads)", 1, false, false, false),
        ("stock again (control)", 1, false, false, false),
        ("eager, staging belt", 1, false, false, true),
        ("eager, shape API", 1, false, true, true),
        ("2 threads", 2, false, true, true),
        ("4 threads", 4, false, true, true),
        ("8 threads", 8, false, true, true),
        ("20 threads", 20, false, true, true),
        ("cache, 1 thread", 1, true, true, true),
        ("cache, 8 threads", 8, true, true, true),
    ];
    let args: Vec<String> = std::env::args().collect();
    let reference_dir = args.iter().position(|a| a == "--reference").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    let mut all_ok = true;
    let mut compared = 0;
    for name in VERIFY_SCENES {
        let mut reference: Option<Vec<u8>> = None;
        let earlier = reference_dir.as_ref().map(|d| common::read_png(&d.join(format!("verify-{name}.png"))));
        if let Some(None) = earlier {
            println!("[verify] {name:10} earlier reference missing");
            all_ok = false;
        }
        for (label, threads, cache, shapes, belt) in configs {
            crate::sparse::SHAPE_API.store(shapes, std::sync::atomic::Ordering::Relaxed);
            off.renderer.set_staging_belt(belt);
            let mut scene = vello_gpu::Scene::new(off.w as u16, off.h as u16);
            scene.set_parallelism(threads);
            scene.set_strip_cache(cache);
            if cache {
                // Frame 1 fills the cache; frame 2 (compared) reuses it.
                encode_named(&mut off, &mut scene, name, 0.5);
                off.render(&mut scene);
                let _ = scene.take_strip_cache_stats();
            }
            encode_named(&mut off, &mut scene, name, 0.5);
            off.render(&mut scene);
            let stats = scene.take_strip_cache_stats();
            let px = off.pixels();
            if let Some(Some((w, h, earlier))) = &earlier {
                let differing = earlier.chunks_exact(4).zip(px.chunks_exact(4)).filter(|(a, b)| a != b).count();
                let ok = differing == 0 && earlier.len() == px.len() && (*w, *h) == (off.w, off.h);
                all_ok &= ok;
                compared += 1;
                println!("[verify] {name:10} {label:20} vs earlier build: {}", if ok { "IDENTICAL".to_owned() } else { format!("DIFFERS in {differing} pixels") });
            }
            match &reference {
                None => {
                    common::write_png(&common::shots_dir().join(format!("verify-{name}.png")), off.w, off.h, &px);
                    reference = Some(px);
                    println!("[verify] {name:10} {label:20} reference");
                }
                Some(r) => {
                    let differing = r.chunks_exact(4).zip(px.chunks_exact(4)).filter(|(a, b)| a != b).count();
                    let ok = differing == 0 && r.len() == px.len();
                    all_ok &= ok;
                    compared += 1;
                    let cache_note = if cache { format!(" (cache hits {}, misses {})", stats.0, stats.1) } else { String::new() };
                    println!("[verify] {name:10} {label:20} {}{cache_note}", if ok { "IDENTICAL".to_owned() } else { format!("DIFFERS in {differing} pixels") });
                }
            }
        }
    }
    crate::sparse::SHAPE_API.store(true, std::sync::atomic::Ordering::Relaxed);
    off.renderer.set_staging_belt(true);
    println!("[verify] {compared} comparisons: {}", if all_ok { "all byte-identical" } else { "MISMATCH" });
    // Former panics (fork turns them into skipped draws / warnings).
    let survived = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut scene = vello_gpu::Scene::new(off.w as u16, off.h as u16);
        scene.set_paint(Brush::Image(ImageBrush { image: pixmap_source(&coverage_image()), sampler: ImageSampler::default() }));
        scene.fill_rect(&Rect::new(10., 10., 74., 74.));
        let mask = vello_gpu::Mask::new_alpha(&vello_gpu::Pixmap::new(64, 64));
        scene.push_layer(None, None, None, Some(mask), None);
        scene.set_paint(Color::BLACK);
        scene.fill_rect(&Rect::new(0., 0., 10., 10.));
        scene.pop_layer();
        off.render(&mut scene);
    }))
    .is_ok();
    println!("[verify] pixmap image source + mask layer: {}", if survived { "no panic (draw skipped / mask ignored, warnings logged)" } else { "PANICKED" });
    all_ok && survived
}

/// Scaling curve: per thread count, canvas 1k and 10k frame costs; then the
/// strip cache on deka's world (animating) and UI (static).
pub fn scaling(frames: usize, counts: &[usize]) {
    let Some(mut off) = Offscreen::new(3200, 2000) else { return };
    let physical = std::process::Command::new("sysctl").args(["-n", "hw.physicalcpu"]).output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
    println!("[scaling] cores: {physical} physical, {} logical", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));
    let mut sizes = common::list_arg("--shapes");
    if sizes.is_empty() {
        sizes = vec![1_000, 10_000];
    }
    for shapes in sizes.into_iter().filter(|&n| n > 0) {
        let name = if shapes % 1000 == 0 { format!("canvas-{}k", shapes / 1000) } else { format!("canvas-{shapes}") };
        let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
        for &threads in counts {
            let mut g = graph::Graph::new(shapes, 1600., 1000., layout);
            let mut scene = vello_gpu::Scene::new(3200, 2000);
            scene.set_parallelism(threads);
            let (mut enc, mut cpu, mut gpu, mut total) = (crate::common::Series::default(), crate::common::Series::default(), crate::common::Series::default(), crate::common::Series::default());
            let u0 = crate::common::usage();
            let t_start = Instant::now();
            for f in 0..frames + 10 {
                g.update(f as f64 / 60.);
                let t0 = Instant::now();
                scene.reset();
                encode_graph(&mut scene, &g, 2.0, 3200., 2000.);
                scene.flush();
                let t1 = Instant::now();
                let (c, gw) = off.render(&mut scene);
                if f >= 10 {
                    enc.push(t1 - t0);
                    cpu.push(c);
                    gpu.push(gw);
                    total.push(t0.elapsed());
                }
            }
            let u1 = crate::common::usage();
            let wall = t_start.elapsed().as_secs_f64();
            let [gen_ns, merge_ns, record_ns] = scene.take_flush_timing();
            let [sched_ns, upload_ns, exec_ns, _] = off.renderer.take_render_timing();
            let per = |ns: u64| ns as f64 / 1e6 / (frames + 10) as f64;
            println!(
                "[scaling] {name:10} threads {threads:2}: strips {:6.2} ms (parallel gen {:5.2}, merge {:4.2}, record {:4.2}) | render() {:5.2} (schedule {:4.2}, alpha upload {:4.2}, execute+strip upload {:4.2}) | GPU {:5.2} | frame {:6.2} ms -> {:5.1} fps offscreen | CPU {:5.1}% of a core",
                enc.mean(), per(gen_ns), per(merge_ns), per(record_ns), cpu.mean(), per(sched_ns), per(upload_ns), per(exec_ns), gpu.mean(), total.mean(), 1000. / total.mean(),
                (u1.cpu_ns - u0.cpu_ns) as f64 / 1e9 / wall * 100.
            );
        }
    }
    if std::env::args().any(|a| a == "--canvas-only") {
        return;
    }
    // Small frames must not get slower with threads: deka's world and UI at
    // 1 thread and at the largest count, strip cache off and on.
    let most = counts.iter().copied().max().unwrap_or(1);
    for name in ["world", "ui"] {
        for (threads, cache) in [(1, false), (most, false), (1, true), (most, true)] {
            let mut scene = vello_gpu::Scene::new(3200, 2000);
            scene.set_parallelism(threads);
            scene.set_strip_cache(cache);
            let mut enc = crate::common::Series::default();
            let mut ren = crate::common::Series::default();
            let mut world = World::new();
            world.start();
            let ui_renderer = deka_native_ui::scene::Renderer::new();
            let host = Host::new(crate::common::UiApp);
            for f in 0..frames + 10 {
                let deka = if name == "world" { world.frame(1600., 1000., f as f64 * 16.7, false) } else { ui_renderer.render_at(&host.render(), 1600., 1000., 2., f as f64 * 16.7, false) };
                off.upload(&deka);
                let t0 = Instant::now();
                scene.reset();
                encode_deka(&mut scene, &deka, 2.0, &off.images);
                scene.flush();
                let t1 = Instant::now();
                let (c, _) = off.render(&mut scene);
                if f >= 10 {
                    enc.push(t1 - t0);
                    ren.push(c);
                }
            }
            let (hits, misses, entries) = scene.take_strip_cache_stats();
            println!("[scaling] {name:5} threads {threads:2} strip cache {:3}: scene encode+strips mean {:5.2} p95 {:5.2} ms | render() mean {:5.2} p95 {:5.2} ms (hits {hits}, misses {misses}, entries {entries})", if cache { "on" } else { "off" }, enc.mean(), enc.p95(), ren.mean(), ren.p95());
        }
    }
}
