//! vello_cpu (multithreaded, SIMD) presented with softbuffer. No GPU API.

use super::*;
use std::num::NonZeroU32;

pub struct CpuPresenter {
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    _context: softbuffer::Context<Arc<Window>>,
    rc: vello_cpu::RenderContext,
    res: vello_cpu::Resources,
    pixmap: vello_cpu::Pixmap,
    images: HashMap<String, ImageSource>,
    width: u32,
    height: u32,
    /// render / colour conversion / softbuffer present, ms per frame.
    split: [crate::common::Series; 3],
}

impl Drop for CpuPresenter {
    fn drop(&mut self) {
        if !self.split[0].0.is_empty() {
            println!(
                "[vello_cpu] per frame: raster {}\n            convert {}\n            present {}",
                self.split[0].summary(),
                self.split[1].summary(),
                self.split[2].summary()
            );
        }
    }
}

fn nz(v: u32) -> NonZeroU32 {
    NonZeroU32::new(v.max(1)).unwrap_or(NonZeroU32::MIN)
}

impl CpuPresenter {
    fn sized(&mut self, w: u32, h: u32) {
        if (w, h) == (self.width, self.height) || w == 0 || h == 0 {
            return;
        }
        if let Err(e) = self.surface.resize(nz(w), nz(h)) {
            eprintln!("[vello_cpu] softbuffer resize: {e}");
        }
        self.rc = vello_cpu::RenderContext::new_with(w as u16, h as u16, vello_cpu::RenderSettings::default());
        self.pixmap = vello_cpu::Pixmap::new(w as u16, h as u16);
        (self.width, self.height) = (w, h);
    }
}

impl Presenter for CpuPresenter {
    const NAME: &'static str = "vello_cpu";

    fn new(window: Arc<Window>, times: &mut StartupTimes) -> Result<Self, String> {
        let t = Instant::now();
        let context = softbuffer::Context::new(window.clone()).map_err(|e| format!("softbuffer context: {e}"))?;
        let mut surface = softbuffer::Surface::new(&context, window.clone()).map_err(|e| format!("softbuffer surface: {e}"))?;
        let size = window.inner_size();
        surface.resize(nz(size.width), nz(size.height)).map_err(|e| format!("softbuffer resize: {e}"))?;
        times.device_ms = t.elapsed().as_secs_f64() * 1000.;
        let t = Instant::now();
        let settings = vello_cpu::RenderSettings::default();
        println!("[vello_cpu] {} worker threads, SIMD {:?}", settings.num_threads, settings.level);
        let rc = vello_cpu::RenderContext::new_with(size.width as u16, size.height as u16, settings);
        times.renderer_ms = t.elapsed().as_secs_f64() * 1000.;
        Ok(Self {
            surface,
            _context: context,
            rc,
            res: vello_cpu::Resources::new(),
            pixmap: vello_cpu::Pixmap::new(size.width as u16, size.height as u16),
            images: HashMap::new(),
            width: size.width,
            height: size.height,
            split: Default::default(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.sized(size.width, size.height);
    }

    fn encode_deka(&mut self, deka: &DekaScene, scale: f64) {
        self.rc.reset();
        let used = used_ids(deka);
        for g in &deka.images {
            if used.contains(g.id.as_str()) && !self.images.contains_key(&g.id) {
                self.images.insert(g.id.clone(), pixmap_source(g));
            }
        }
        self.images.retain(|id, _| used.contains(id.as_str()));
        encode_deka(&mut self.rc, deka, scale, &self.images);
    }

    fn encode_graph(&mut self, graph: &graph::Graph, scale: f64) {
        self.rc.reset();
        encode_graph(&mut self.rc, graph, scale, self.width as f64, self.height as f64);
    }

    fn present(&mut self) -> Result<Presented, String> {
        let t0 = Instant::now();
        self.rc.flush();
        self.rc.render(&mut self.pixmap, &mut self.res);
        let t1 = Instant::now();
        let mut buffer = self.surface.buffer_mut().map_err(|e| format!("softbuffer buffer: {e}"))?;
        let src = self.pixmap.data_as_u8_slice();
        if buffer.len() * 4 != src.len() {
            return Ok(Presented::Retry);
        }
        // Premultiplied RGBA over an opaque background -> 0RGB.
        for (dst, px) in buffer.iter_mut().zip(src.chunks_exact(4)) {
            *dst = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
        let t2 = Instant::now();
        buffer.present().map_err(|e| format!("softbuffer present: {e}"))?;
        self.split[0].push(t1 - t0);
        self.split[1].push(t2 - t1);
        self.split[2].push(t2.elapsed());
        Ok(Presented::Done)
    }

    fn screenshot(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        Some((self.width, self.height, self.pixmap.data_as_u8_slice().to_vec()))
    }
}

/// Coverage sheet offscreen. Blend modes are probed one by one so a mode
/// that panics is reported instead of aborting the sheet.
pub fn coverage_offscreen() {
    let image = pixmap_source(&coverage_image());
    let mut failing = vec![];
    for (name, mix, compose) in BLEND_MODES {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut rc = vello_cpu::RenderContext::new(64, 64);
            let mut res = vello_cpu::Resources::new();
            let cell = Rect::new(0., 0., 64., 64.);
            rc.push_layer(Some(&cell.to_path(TOLERANCE)), None, None, None, None);
            rc.set_paint(Color::BLACK);
            rc.fill_rect(&Rect::new(0., 0., 32., 32.));
            rc.push_layer(Some(&cell.to_path(TOLERANCE)), Some(BlendMode::new(mix, compose)), None, None, None);
            rc.set_paint(Color::WHITE);
            rc.fill_rect(&Rect::new(16., 16., 48., 48.));
            rc.pop_layer();
            rc.pop_layer();
            rc.flush();
            let mut pm = vello_cpu::Pixmap::new(64, 64);
            rc.render(&mut pm, &mut res);
        }))
        .is_ok();
        if !ok {
            failing.push(name);
        }
    }
    println!("[vello_cpu] blend modes that panic: {failing:?}");
    let (w, h) = COVERAGE;
    let mut rc = vello_cpu::RenderContext::new(w as u16, h as u16);
    let mut res = vello_cpu::Resources::new();
    coverage(&mut rc, &mut res, &image, &failing);
    rc.flush();
    let mut pm = vello_cpu::Pixmap::new(w as u16, h as u16);
    rc.render(&mut pm, &mut res);
    common::write_png(&common::shots_dir().join("coverage-vello_cpu.png"), w, h, pm.data_as_u8_slice());
    println!("[vello_cpu] wrote coverage-vello_cpu.png");
    let (w, h) = FILTER_SHEET;
    let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // vello_cpu 0.3: filters are unimplemented in multi-threaded mode (panics); one thread works.
        let settings = vello_cpu::RenderSettings { num_threads: 0, ..Default::default() };
        let mut rc = vello_cpu::RenderContext::new_with(w as u16, h as u16, settings);
        let mut res = vello_cpu::Resources::new();
        filter_sheet(&mut rc, &mut res);
        rc.flush();
        let mut pm = vello_cpu::Pixmap::new(w as u16, h as u16);
        rc.render(&mut pm, &mut res);
        common::write_png(&common::shots_dir().join("filters-vello_cpu.png"), w, h, pm.data_as_u8_slice());
    }));
    println!("[vello_cpu] filter sheet: {}", if ok.is_ok() { "rendered" } else { "PANICKED" });
}

/// vello_cpu presented through wgpu: upload the pixmap to a texture each frame
/// and blit it to the surface (no colour-conversion loop, no CGImage copy).
#[cfg(feature = "cpu-wgpu")]
pub struct CpuWgpuPresenter {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    blitter: wgpu::util::TextureBlitter,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    rc: vello_cpu::RenderContext,
    res: vello_cpu::Resources,
    pixmap: vello_cpu::Pixmap,
    images: HashMap<String, ImageSource>,
    split: [crate::common::Series; 2],
}

#[cfg(feature = "cpu-wgpu")]
impl Drop for CpuWgpuPresenter {
    fn drop(&mut self) {
        if !self.split[0].0.is_empty() {
            println!("[vello_cpu+wgpu] per frame: raster {}\n                 upload+blit+present {}", self.split[0].summary(), self.split[1].summary());
        }
    }
}

#[cfg(feature = "cpu-wgpu")]
fn upload_texture(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vello_cpu frame"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

#[cfg(feature = "cpu-wgpu")]
impl Presenter for CpuWgpuPresenter {
    const NAME: &'static str = "vello_cpu+wgpu";

    fn new(window: Arc<Window>, times: &mut StartupTimes) -> Result<Self, String> {
        let t = Instant::now();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let surface = instance.create_surface(window.clone()).map_err(|e| format!("surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() })).map_err(|e| format!("adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).map_err(|e| format!("device: {e}"))?;
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
        surface.configure(&device, &config);
        let blitter = wgpu::util::TextureBlitter::new(&device, format);
        let (texture, view) = upload_texture(&device, size.width, size.height);
        times.device_ms = t.elapsed().as_secs_f64() * 1000.;
        let t = Instant::now();
        let rc = vello_cpu::RenderContext::new_with(size.width as u16, size.height as u16, vello_cpu::RenderSettings::default());
        times.renderer_ms = t.elapsed().as_secs_f64() * 1000.;
        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            blitter,
            texture,
            view,
            rc,
            res: vello_cpu::Resources::new(),
            pixmap: vello_cpu::Pixmap::new(size.width as u16, size.height as u16),
            images: HashMap::new(),
            split: Default::default(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 || (size.width, size.height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        (self.texture, self.view) = upload_texture(&self.device, size.width, size.height);
        self.rc = vello_cpu::RenderContext::new_with(size.width as u16, size.height as u16, vello_cpu::RenderSettings::default());
        self.pixmap = vello_cpu::Pixmap::new(size.width as u16, size.height as u16);
    }

    fn encode_deka(&mut self, deka: &DekaScene, scale: f64) {
        self.rc.reset();
        let used = used_ids(deka);
        for g in &deka.images {
            if used.contains(g.id.as_str()) && !self.images.contains_key(&g.id) {
                self.images.insert(g.id.clone(), pixmap_source(g));
            }
        }
        self.images.retain(|id, _| used.contains(id.as_str()));
        encode_deka(&mut self.rc, deka, scale, &self.images);
    }

    fn encode_graph(&mut self, graph: &graph::Graph, scale: f64) {
        self.rc.reset();
        encode_graph(&mut self.rc, graph, scale, self.config.width as f64, self.config.height as f64);
    }

    fn present(&mut self) -> Result<Presented, String> {
        let t0 = Instant::now();
        self.rc.flush();
        self.rc.render(&mut self.pixmap, &mut self.res);
        let t1 = Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(Presented::Occluded),
            _ => {
                self.surface.configure(&self.device, &self.config);
                return Ok(Presented::Retry);
            }
        };
        let (w, h) = (self.config.width, self.config.height);
        self.queue.write_texture(
            self.texture.as_image_copy(),
            self.pixmap.data_as_u8_slice(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let target = frame.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.blitter.copy(&self.device, &mut enc, &self.view, &target);
        self.queue.submit([enc.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        self.split[0].push(t1 - t0);
        self.split[1].push(t1.elapsed());
        Ok(Presented::Done)
    }

    fn screenshot(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        Some((self.config.width, self.config.height, self.pixmap.data_as_u8_slice().to_vec()))
    }
}

/// Offscreen: what presenting vello_cpu output through wgpu costs on this
/// machine (raster, then upload 3200x2000 RGBA + blit + wait), without a window.
#[cfg(feature = "cpu-wgpu")]
pub fn upload_bench(frames: usize) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else { return };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default())) else { return };
    let (w, h) = (3200u32, 2000u32);
    let (texture, view) = upload_texture(&device, w, h);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
    let blitter = wgpu::util::TextureBlitter::new(&device, wgpu::TextureFormat::Bgra8Unorm);
    let mut rc = vello_cpu::RenderContext::new_with(w as u16, h as u16, vello_cpu::RenderSettings::default());
    let mut res = vello_cpu::Resources::new();
    let mut pixmap = vello_cpu::Pixmap::new(w as u16, h as u16);
    println!("[vello_cpu+wgpu bench] adapter {} ({} worker threads)", adapter.get_info().name, vello_cpu::RenderSettings::default().num_threads);
    for shapes in [1_000usize, 10_000] {
        let layout = if shapes <= 1_000 { graph::Layout::Force } else { graph::Layout::Laid };
        let mut g = graph::Graph::new(shapes, 1600., 1000., layout);
        let (mut raster, mut upload) = (crate::common::Series::default(), crate::common::Series::default());
        for f in 0..frames + 10 {
            g.update(f as f64 / 60.);
            let t0 = Instant::now();
            rc.reset();
            encode_graph(&mut rc, &g, 2.0, w as f64, h as f64);
            rc.flush();
            rc.render(&mut pixmap, &mut res);
            let t1 = Instant::now();
            queue.write_texture(
                texture.as_image_copy(),
                pixmap.data_as_u8_slice(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            let mut enc = device.create_command_encoder(&Default::default());
            blitter.copy(&device, &mut enc, &view, &target_view);
            queue.submit([enc.finish()]);
            if let Err(e) = device.poll(wgpu::PollType::wait_indefinitely()) {
                eprintln!("poll: {e}");
            }
            if f >= 10 {
                raster.push(t1 - t0);
                upload.push(t1.elapsed());
            }
        }
        println!("[vello_cpu+wgpu bench] canvas {shapes}: raster {}\n                         upload+blit+wait {}", raster.summary(), upload.summary());
    }
}
