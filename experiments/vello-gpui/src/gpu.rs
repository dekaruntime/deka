//! wgpu + vello setup, the render target, CPU readback, and the
//! RGBA -> NV12 (4:2:0 Y'CbCr, BT.601 full range) conversion pass that GPUI
//! 0.2.2's macOS surface element requires.

use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderer: Renderer,
}

impl Gpu {
    pub fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .expect("no wgpu adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vello-spike"),
            required_features: adapter.features()
                & (wgpu::Features::CLEAR_TEXTURE | wgpu::Features::PIPELINE_CACHE),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .expect("no wgpu device");
        let renderer = Renderer::new(
            &device,
            RendererOptions {
                antialiasing_support: AaSupport::area_only(),
                ..Default::default()
            },
        )
        .expect("vello renderer");
        Gpu {
            instance,
            adapter,
            device,
            queue,
            renderer,
        }
    }

    pub fn adapter_line(&self) -> String {
        let i = self.adapter.get_info();
        format!("{} ({:?}, driver {:?})", i.name, i.backend, i.driver_info)
    }

    /// Encode + submit vello's compute pipeline into `target`.
    pub fn render(&mut self, scene: &Scene, target: &Target, base: vello::peniko::Color) {
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &target.view,
                &RenderParams {
                    base_color: base,
                    width: target.width,
                    height: target.height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .expect("vello render");
    }

    /// Block until everything submitted so far has finished on the GPU.
    pub fn wait(&self) {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
    }
}

/// vello renders with compute, so its target must be an Rgba8Unorm storage
/// texture. COPY_SRC for readback, TEXTURE_BINDING for the NV12 pass.
pub struct Target {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Target {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vello-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Target {
            texture,
            view,
            width,
            height,
        }
    }
}

/// Persistent staging buffer for GPU -> CPU readback of a `Target`.
pub struct Readback {
    buffer: wgpu::Buffer,
    padded_row: u32,
    width: u32,
    height: u32,
}

impl Readback {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let padded_row = (width * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Readback {
            buffer,
            padded_row,
            width,
            height,
        }
    }

    /// Copy `target` into the staging buffer, wait, and hand the tightly
    /// packed RGBA rows to `f` (which may swizzle while copying out).
    pub fn read(&self, gpu: &Gpu, target: &Target, out: &mut Vec<u8>, bgra: bool) {
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([enc.finish()]);
        let slice = self.buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        gpu.wait();
        {
            let data = slice.get_mapped_range().expect("mapped range");
            let row = (self.width * 4) as usize;
            out.clear();
            out.reserve(row * self.height as usize);
            for y in 0..self.height as usize {
                let start = y * self.padded_row as usize;
                out.extend_from_slice(&data[start..start + row]);
            }
        }
        self.buffer.unmap();
        if bgra {
            for px in out.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
    }
}

const NV12_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;

struct VOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    // Fullscreen triangle.
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return o;
}

// BT.601 full range, the inverse of the matrix in GPUI's surface_fragment.
@fragment
fn fs_y(v: VOut) -> @location(0) vec4<f32> {
    let c = textureLoad(src, vec2<i32>(v.pos.xy), 0).rgb;
    return vec4<f32>(dot(c, vec3<f32>(0.299, 0.587, 0.114)), 0.0, 0.0, 1.0);
}

@fragment
fn fs_cbcr(v: VOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(v.pos.xy) * 2;
    let c = 0.25 * (textureLoad(src, p, 0).rgb + textureLoad(src, p + vec2<i32>(1, 0), 0).rgb
        + textureLoad(src, p + vec2<i32>(0, 1), 0).rgb + textureLoad(src, p + vec2<i32>(1, 1), 0).rgb);
    let cb = dot(c, vec3<f32>(-0.168736, -0.331264, 0.5)) + 0.5;
    let cr = dot(c, vec3<f32>(0.5, -0.418688, -0.081312)) + 0.5;
    return vec4<f32>(cb, cr, 0.0, 1.0);
}
"#;

/// Two tiny render passes: full-res luma into an R8Unorm plane, half-res
/// chroma into an Rg8Unorm plane. Both planes are textures over one IOSurface.
pub struct Nv12Pass {
    y_pipeline: wgpu::RenderPipeline,
    cbcr_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl Nv12Pass {
    pub fn new(device: &wgpu::Device, source: &Target) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12"),
            source: wgpu::ShaderSource::Wgsl(NV12_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let y_pipeline = pipeline("fs_y", wgpu::TextureFormat::R8Unorm);
        let cbcr_pipeline = pipeline("fs_cbcr", wgpu::TextureFormat::Rg8Unorm);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source.view),
            }],
        });
        Nv12Pass {
            y_pipeline,
            cbcr_pipeline,
            bind_group,
        }
    }

    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, y: &wgpu::TextureView, cbcr: &wgpu::TextureView) {
        for (view, pipeline) in [(y, &self.y_pipeline), (cbcr, &self.cbcr_pipeline)] {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

pub fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("mkdir shots");
    }
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(rgba).expect("png data");
}
