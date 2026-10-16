//! The same graph scenes through vello's *sparse strips* renderers, which need
//! no compute shaders: `vello_cpu` (pure CPU, SIMD + threads) and `vello_gpu`
//! (CPU path processing, GPU rasterisation with vertex/fragment shaders; also
//! has a WebGL2 backend). Feature `sparse`.

use std::time::Instant;

use vello::kurbo::{Affine, Rect, Shape, Stroke};
use vello::peniko::Color;
use vello::wgpu;

use crate::gpu::{Gpu, Readback, Target, write_png};
use crate::graph::{BACKGROUND, Graph, Layout, Painter};
use crate::stats::Series;

const TOLERANCE: f64 = 0.1;

struct CpuPainter<'a> {
    ctx: &'a mut vello_cpu::RenderContext,
}

impl Painter for CpuPainter<'_> {
    fn fill(&mut self, shape: &impl Shape, color: Color) {
        self.ctx.set_paint(color);
        self.ctx.fill_path(&shape.to_path(TOLERANCE));
    }
    fn stroke(&mut self, shape: &impl Shape, style: &Stroke, color: Color) {
        self.ctx.set_paint(color);
        self.ctx.set_stroke(style.clone());
        self.ctx.stroke_path(&shape.to_path(TOLERANCE));
    }
}

struct GpuPainter<'a> {
    scene: &'a mut vello_gpu::Scene,
}

impl Painter for GpuPainter<'_> {
    fn fill(&mut self, shape: &impl Shape, color: Color) {
        self.scene.set_paint(color);
        self.scene.fill_path(&shape.to_path(TOLERANCE));
    }
    fn stroke(&mut self, shape: &impl Shape, style: &Stroke, color: Color) {
        self.scene.set_paint(color);
        self.scene.set_stroke(style.clone());
        self.scene.stroke_path(&shape.to_path(TOLERANCE));
    }
}

pub fn run(frames: usize, shapes_list: &[usize], shots: &std::path::Path) {
    let (w, h, scale) = (3200u16, 2000u16, 2.0f64);
    let gpu = Gpu::new();
    println!("adapter: {}", gpu.adapter_line());
    let settings = vello_cpu::RenderSettings::default();
    println!("vello_cpu: {} worker threads, SIMD {:?}", settings.num_threads, settings.level);
    let mut cpu_ctx = vello_cpu::RenderContext::new_with(w, h, settings);
    let mut cpu_res = vello_cpu::Resources::new();
    let mut pixmap = vello_cpu::Pixmap::new(w, h);

    // vello_gpu renders with a render pipeline, into a RENDER_ATTACHMENT texture.
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vello_gpu target"),
        size: wgpu::Extent3d { width: w.into(), height: h.into(), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let (mut renderer, mut gpu_res) = vello_gpu::Renderer::new(
        &gpu.device,
        &vello_gpu::RenderTargetConfig { format: texture.format(), width: w, height: h },
    );
    let size = vello_gpu::RenderSize { width: w, height: h };
    let depth = vello_gpu::Renderer::create_depth_texture_view(&gpu.device, &size);
    let bindings = vello_gpu::TextureBindings::new();
    let mut gpu_scene = vello_gpu::Scene::new(w, h);
    let readback_target = Target { texture: texture.clone(), view: view.clone(), width: w.into(), height: h.into() };
    let readback = Readback::new(&gpu.device, w.into(), h.into());
    let bg = Rect::new(0.0, 0.0, w as f64, h as f64);

    for &shapes in shapes_list {
        let layout = if shapes <= 1_000 { Layout::Force } else { Layout::Laid };
        let mut graph = Graph::new(shapes, w as f64 / scale, h as f64 / scale, layout);
        let (mut cpu_t, mut gpu_cpu_t, mut gpu_t) = (Series::default(), Series::default(), Series::default());
        for f in 0..frames + 30 {
            graph.update(f as f64 / 60.0);

            // vello_cpu: encode + rasterise into a CPU pixmap.
            let t0 = Instant::now();
            cpu_ctx.reset();
            cpu_ctx.set_transform(Affine::IDENTITY);
            cpu_ctx.set_paint(BACKGROUND);
            cpu_ctx.fill_rect(&bg);
            cpu_ctx.set_transform(Affine::scale(scale));
            graph.paint(&mut CpuPainter { ctx: &mut cpu_ctx });
            cpu_ctx.flush();
            cpu_ctx.render(&mut pixmap, &mut cpu_res);
            let t1 = Instant::now();

            // vello_gpu: CPU strips + GPU raster.
            gpu_scene.reset();
            gpu_scene.set_transform(Affine::IDENTITY);
            gpu_scene.set_paint(BACKGROUND);
            gpu_scene.fill_rect(&bg);
            gpu_scene.set_transform(Affine::scale(scale));
            graph.paint(&mut GpuPainter { scene: &mut gpu_scene });
            let t2 = Instant::now();
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            renderer
                .render(
                    &gpu_scene,
                    &mut gpu_res,
                    &gpu.device,
                    &gpu.queue,
                    &mut enc,
                    &size,
                    &view,
                    Some(&depth),
                    &bindings,
                    vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()),
                )
                .expect("vello_gpu render");
            gpu.queue.submit([enc.finish()]);
            gpu.wait();
            let t3 = Instant::now();
            if f >= 30 {
                cpu_t.push(t1 - t0);
                gpu_cpu_t.push(t2 - t1);
                gpu_t.push(t3 - t1);
            }
        }
        // Screenshots of both.
        let rgba: Vec<u8> = pixmap
            .data()
            .iter()
            .flat_map(|p| [p.r, p.g, p.b, p.a])
            .collect();
        write_png(&shots.join(format!("sparse-cpu-{shapes}.png")), w.into(), h.into(), &rgba);
        let mut out = Vec::new();
        readback.read(&gpu, &readback_target, &mut out, false);
        write_png(&shots.join(format!("sparse-gpu-{shapes}.png")), w.into(), h.into(), &out);
        println!("== sparse strips, shapes {shapes} ({layout:?}), {w}x{h}, {frames} frames");
        println!("vello_cpu encode+render (no GPU at all)  {}", cpu_t.summary());
        println!("vello_gpu scene encode (CPU part)        {}", gpu_cpu_t.summary());
        println!("vello_gpu encode+render+wait             {}", gpu_t.summary());
    }
}
