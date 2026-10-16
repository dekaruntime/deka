//! Size probe. Build twice with deka's `native` profile and compare:
//!   cargo build --profile native --bin size-probe --no-default-features   (GPUI only)
//!   cargo build --profile native --bin size-probe                         (GPUI + vello + wgpu)
//! The vello build really links the renderer: device, pipelines, a scene, a
//! render to a texture. It is never run by the spike; only its bytes matter.

use gpui::{App, AppContext, Application, Context, IntoElement, Render, Window, WindowOptions, div};

struct Probe;
impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[cfg(feature = "vello")]
fn use_vello() {
    use vello::kurbo::{Affine, Circle, Stroke};
    use vello::peniko::{Color, Fill};
    use vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene, wgpu};
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut renderer = Renderer::new(&device, RendererOptions::default()).unwrap();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let mut scene = Scene::new();
    let c = Circle::new((32.0, 32.0), 20.0);
    scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None, &c);
    scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, Color::BLACK, None, &c);
    renderer
        .render_to_texture(
            &device,
            &queue,
            &scene,
            &texture.create_view(&Default::default()),
            &RenderParams { base_color: Color::BLACK, width: 64, height: 64, antialiasing_method: AaConfig::Area },
        )
        .unwrap();
}

fn main() {
    if std::env::args().nth(1).as_deref() != Some("--really-run") {
        println!("size probe: built only to be measured");
        return;
    }
    #[cfg(feature = "vello")]
    use_vello();
    Application::new().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Probe)).unwrap();
    });
}
