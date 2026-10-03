//! The GPU side: one wgpu device and deka's vello_gpu fork, drawing deka scenes
//! into a window surface or an offscreen texture. Every failure is an `Err`:
//! the caller logs it and skips the frame.
use super::encode::{self, Images, Tile};
use crate::scene::Scene;
use std::sync::mpsc;
use std::thread::JoinHandle;

/// Non-sRGB BGRA, like GPUI's Metal layer: colours are sRGB values, blended as such.
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
/// Formats the renderer can target, in order of preference (never an sRGB view:
/// deka's colours are already sRGB-encoded).
const FORMATS: [wgpu::TextureFormat; 2] = [FORMAT, wgpu::TextureFormat::Rgba8Unorm];
/// Glyph images are small; pages grow on demand up to `max_atlases`.
const ATLAS: (u16, u16) = (encode::MAX_TILE, encode::MAX_TILE);

/// Worker threads for path strips: physical (on Apple Silicon, performance)
/// cores. Hyper-threads and efficiency cores add little (spike phase 4).
pub(crate) fn strip_threads() -> usize {
    #[cfg(target_os = "macos")]
    if let Some(n) = sysctl_usize(c"hw.perflevel0.physicalcpu") {
        return n.max(1);
    }
    num_cpus::get_physical().max(1)
}

#[cfg(target_os = "macos")]
fn sysctl_usize(name: &std::ffi::CStr) -> Option<usize> {
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: `name` is NUL-terminated; `value` and `size` describe a writable
    // c_int, which is the type of the hw.* core counts.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&raw mut value).cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && value > 0).then_some(value as usize)
}

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// The device, the vello_gpu renderer and the uploaded images.
pub(crate) struct Gpu {
    pub(crate) instance: wgpu::Instance,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    images: Images,
    scene: vello_gpu::Scene,
}

impl Gpu {
    /// Create everything that does not need a window. Safe on any thread.
    pub(crate) fn new() -> Result<Self, String> {
        Self::with_instance(instance(), None)
    }

    fn with_instance(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, String> {
        super::trace::mark("gpu: instance");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        super::trace::mark("gpu: adapter");
        // vello_gpu needs no compute shaders and fits WebGL2-class limits, so a
        // device that cannot offer the defaults still gets a renderer.
        let device = [
            wgpu::Limits::default(),
            wgpu::Limits::downlevel_webgl2_defaults(),
        ]
        .into_iter()
        .find_map(|limits| {
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("deka"),
                required_limits: limits.using_resolution(adapter.limits()),
                ..Default::default()
            }))
            .ok()
        });
        let (device, queue) = device.ok_or("the GPU adapter refused a device")?;
        super::trace::mark("gpu: device");
        // An uncaptured wgpu error would panic; log it and keep the window alive.
        device.on_uncaptured_error(std::sync::Arc::new(|error| {
            eprintln!("deka: GPU error: {error}");
        }));
        let (renderer, resources) = renderer(&device, FORMAT);
        super::trace::mark("gpu: renderer");
        let mut scene = vello_gpu::Scene::new(1, 1);
        scene.set_parallelism(strip_threads());
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            format: FORMAT,
            renderer,
            resources,
            images: Images::default(),
            scene,
        })
    }

    /// Start [`Gpu::new`] on its own thread (from `main`, before the window).
    pub(crate) fn start() -> Pending {
        let (send, receive) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("deka-gpu-init".into())
            .spawn(move || {
                super::trace::mark("gpu: thread");
                let _ = send.send(Gpu::new());
            })
            .ok();
        Pending { thread, receive }
    }

    /// The surface's best format this renderer can draw into; rebuilds the
    /// renderer's pipelines if it differs from the one it was created for.
    pub(crate) fn adopt(
        &mut self,
        surface: &wgpu::Surface<'_>,
    ) -> Result<wgpu::TextureFormat, String> {
        if !self.adapter.is_surface_supported(surface) {
            *self = Self::with_instance(self.instance.clone(), Some(surface))?;
        }
        let formats = surface.get_capabilities(&self.adapter).formats;
        let format = FORMATS
            .into_iter()
            .find(|f| formats.contains(f))
            .ok_or_else(|| format!("the window offers no BGRA/RGBA format: {formats:?}"))?;
        if format != self.format {
            // New pipelines come with a new atlas: everything is uploaded again.
            self.images = Images::default();
            (self.renderer, self.resources) = renderer(&self.device, format);
            self.format = format;
        }
        Ok(format)
    }

    /// Draw `scene` at `scale` into `view` (`width` x `height` device pixels).
    pub(crate) fn draw(
        &mut self,
        scene: &Scene,
        scale: f64,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let (Ok(w), Ok(h)) = (u16::try_from(width), u16::try_from(height)) else {
            return Err(format!("{width}x{height} exceeds the renderer's size"));
        };
        if w == 0 || h == 0 {
            return Ok(());
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("deka frame"),
            });
        self.sync_images(scene, &mut encoder);
        self.scene.reset_and_resize(w, h);
        encode::encode(&mut self.scene, scene, scale, &self.images);
        self.scene.flush();
        self.renderer
            .render(
                &self.scene,
                &mut self.resources,
                &self.device,
                &self.queue,
                &mut encoder,
                &vello_gpu::RenderSize {
                    width: w,
                    height: h,
                },
                view,
                None,
                &vello_gpu::TextureBindings::new(),
                vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()),
            )
            .map_err(|e| format!("render: {e}"))?;
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Upload the scene's new images and drop the ones it stopped drawing.
    fn sync_images(&mut self, scene: &Scene, encoder: &mut wgpu::CommandEncoder) {
        let (missing, stale) = self.images.plan(scene);
        for id in stale {
            for tile in self.images.remove(&id) {
                self.renderer
                    .destroy_image(&mut self.resources, encoder, tile.id);
            }
        }
        for image in missing {
            let mut uploaded = vec![];
            for (x, y, pixmap) in encode::tiles(image) {
                match self.renderer.try_upload_image(
                    &mut self.resources,
                    &self.device,
                    &self.queue,
                    encoder,
                    &pixmap,
                ) {
                    Ok(id) => uploaded.push(Tile {
                        id,
                        x,
                        y,
                        width: pixmap.width(),
                        height: pixmap.height(),
                    }),
                    Err(e) => eprintln!("deka: image {} not drawn: {e}", image.id),
                }
            }
            self.images.insert(image.id.clone(), uploaded);
        }
    }

    /// Render `scene` offscreen at `scale` and read back straight RGBA rows.
    pub(crate) fn snapshot(&mut self, scene: &Scene, scale: f64) -> Result<Snapshot, String> {
        let width = (f64::from(scene.width) * scale).round().max(1.) as u32;
        let height = (f64::from(scene.height) * scale).round().max(1.) as u32;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("deka snapshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.draw(scene, scale, &view, width, height)?;
        let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("deka snapshot readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (send, receive) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| drop(send.send(r)));
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("readback: {e}"))?;
        receive
            .recv()
            .map_err(|e| format!("readback: {e}"))?
            .map_err(|e| format!("readback: {e}"))?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        {
            let data = buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|e| format!("readback: {e:?}"))?;
            for row in data.chunks_exact(padded as usize) {
                rgba.extend_from_slice(&row[..(width * 4) as usize]);
            }
        }
        if self.format == wgpu::TextureFormat::Bgra8Unorm {
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        Ok(Snapshot {
            width,
            height,
            rgba,
        })
    }
}

fn renderer(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (vello_gpu::Renderer, vello_gpu::Resources) {
    let mut settings = vello_gpu::RenderSettings::default();
    settings.memory_settings.image_atlas_config.atlas_size = ATLAS;
    vello_gpu::Renderer::new_with(
        device,
        &vello_gpu::RenderTargetConfig {
            format,
            width: 1600,
            height: 1000,
        },
        settings,
    )
}

/// A [`Gpu`] being created on its own thread.
pub(crate) struct Pending {
    thread: Option<JoinHandle<()>>,
    receive: mpsc::Receiver<Result<Gpu, String>>,
}

impl Pending {
    /// Wait for the thread; if it could not start, create the GPU here.
    pub(crate) fn join(self) -> Result<Gpu, String> {
        let Some(thread) = self.thread else {
            return Gpu::new();
        };
        let result = self
            .receive
            .recv()
            .unwrap_or_else(|_| Err("GPU start-up thread stopped".into()));
        let _ = thread.join();
        result
    }
}

/// An offscreen render: straight RGBA rows, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Snapshot {
    /// The pixel at `(x, y)` in device pixels.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}
