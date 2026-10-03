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
        let (renderer, resources) = vello_gpu::Renderer::new_with(
            &device,
            &vello_gpu::RenderTargetConfig { format: FORMAT, width: size.width as u16, height: size.height as u16 },
            settings,
        );
        let no_depth = std::env::args().any(|a| a == "--no-depth");
        // Without depth testing, keep only a 1x1 placeholder (the real one is ~25 MB at 3200x2000).
        let depth_size = if no_depth { vello_gpu::RenderSize { width: 1, height: 1 } } else { vello_gpu::RenderSize { width: size.width as u16, height: size.height as u16 } };
        let depth = vello_gpu::Renderer::create_depth_texture_view(&device, &depth_size);
        times.renderer_ms = t.elapsed().as_secs_f64() * 1000.;
        Ok(Self {
            scene: vello_gpu::Scene::new(size.width as u16, size.height as u16),
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
        self.scene = vello_gpu::Scene::new(rs.width, rs.height);
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
    let render = |scene: &vello_gpu::Scene, renderer: &mut vello_gpu::Renderer, resources: &mut vello_gpu::Resources| {
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
            let mut scene = vello_gpu::Scene::new(w as u16, h as u16);
            let cell = Rect::new(0., 0., 64., 64.);
            scene.push_layer(Some(&cell.to_path(TOLERANCE)), None, None, None, None);
            scene.set_paint(Color::BLACK);
            scene.fill_rect(&Rect::new(0., 0., 32., 32.));
            scene.push_layer(Some(&cell.to_path(TOLERANCE)), Some(BlendMode::new(mix, compose)), None, None, None);
            scene.set_paint(Color::WHITE);
            scene.fill_rect(&Rect::new(16., 16., 48., 48.));
            scene.pop_layer();
            scene.pop_layer();
            render(&scene, &mut renderer, &mut resources)
        }));
        if !matches!(ok, Ok(Ok(()))) {
            failing.push(name);
        }
    }
    println!("[vello_gpu {label}] blend modes that panic or error: {failing:?}");
    let mut scene = vello_gpu::Scene::new(w as u16, h as u16);
    coverage(&mut scene, &mut resources, &image, &failing);
    match render(&scene, &mut renderer, &mut resources) {
        Ok(()) => {
            let rgba = readback(&device, &queue, &texture, w, h);
            common::write_png(&common::shots_dir().join(format!("coverage-vello_gpu-{label}.png")), w, h, &rgba);
            println!("[vello_gpu {label}] wrote coverage-vello_gpu-{label}.png");
        }
        Err(e) => println!("[vello_gpu {label}] coverage render failed: {e}"),
    }
    // Filter effects (drop shadow on a path, blurred text).
    let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut scene = vello_gpu::Scene::new(w as u16, h as u16);
        filter_sheet(&mut scene, &mut resources);
        render(&scene, &mut renderer, &mut resources)
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
        let mut scene = vello_gpu::Scene::new(w as u16, h as u16);
        encode_graph(&mut scene, &g, 2.0, w as f64, h as f64);
        let r = render(&scene, &mut renderer, &mut resources);
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
