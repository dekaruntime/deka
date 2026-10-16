//! Backend A: deka main's GPUI glue, copied from crates/deka_native_ui/src/gpu.rs
//! and src/world/gpu.rs (minus rodio audio, identical drawing), plus the
//! measurement protocol. deka's own code is not modified.

use crate::common::{self, App as AppKind, Args, Protocol, Step, WINDOW_H, WINDOW_W};
use deka_native_ui::{Host, scene::Scene, world::World};
use gpui::{prelude::*, *};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc, time::Instant};

/// Verbatim copy of deka_native_ui::gpu::paint_scene (it is pub(crate)).
fn paint_scene(
    bounds: Bounds<Pixels>,
    scene: &Scene,
    window: &mut Window,
    cache: &mut HashMap<String, Arc<RenderImage>>,
) {
    window.paint_quad(fill(bounds, rgb(scene.background)));
    let mut used = std::collections::HashSet::new();
    for paint in &scene.paint {
        if paint.opacity <= 0. {
            continue;
        }
        let rect = Bounds {
            origin: bounds.origin + point(px(paint.rect.x), px(paint.rect.y)),
            size: size(px(paint.rect.width), px(paint.rect.height)),
        };
        let c = paint.clip;
        let mask = ContentMask {
            bounds: Bounds {
                origin: bounds.origin + point(px(c.x), px(c.y)),
                size: size(px(c.width), px(c.height)),
            },
        };
        window.with_content_mask(Some(mask), |window| {
            if let Some(id) = &paint.image {
                let alpha = (paint.opacity * 255.).round() as u8;
                let key = format!("{id}@{alpha}");
                used.insert(key.clone());
                if let Some(glyph) = scene.images.iter().find(|g| &g.id == id) {
                    let image = cache.entry(key).or_insert_with(|| {
                        let mut bgra = glyph.rgba.clone();
                        for pixel in bgra.chunks_exact_mut(4) {
                            pixel.swap(0, 2);
                            pixel[3] = (u16::from(pixel[3]) * u16::from(alpha) / 255) as u8;
                        }
                        let rgba = image::RgbaImage::from_raw(glyph.width as u32, glyph.height as u32, bgra)
                            .unwrap_or_default();
                        Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]))
                    });
                    let _ = window.paint_image(rect, Corners::default(), image.clone(), 0, false);
                }
            } else {
                window.paint_quad(
                    fill(rect, rgba((paint.color << 8) | (paint.opacity * 255.).round() as u32))
                        .corner_radii(px(paint.radius)),
                );
            }
        });
    }
    cache.retain(|id, _| used.contains(id));
}

enum Content {
    World(Rc<RefCell<World>>),
    Ui {
        host: Rc<RefCell<Host<common::UiApp>>>,
        renderer: Rc<deka_native_ui::scene::Renderer>,
        scene: Rc<RefCell<Scene>>,
    },
}

struct View {
    content: Content,
    clock: Instant,
    images: Rc<RefCell<HashMap<String, Arc<RenderImage>>>>,
    protocol: Rc<RefCell<Protocol>>,
    quitting: bool,
}

impl Render for View {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame_start = Instant::now();
        let frame_index = self.protocol.borrow().frame_index();
        if frame_index == 0 {
            common::trace("first render()");
            if !self.protocol.borrow().first_frame_done() {
                common::mark("first View::render() (frame 1)");
            }
        } else if frame_index == 1 && common::timeline_mode() {
            common::mark("second View::render() (frame 1 has been presented)");
        }
        let step = self.protocol.borrow_mut().begin_frame();
        if matches!(step, Step::Quit) || self.quitting {
            common::print_timeline("gpui");
            cx.quit();
            return div().size_full().into_any_element();
        }
        if self.protocol.borrow().args.resize {
            let (w, h) = common::resize_size(self.protocol.borrow().frame_index());
            window.resize(size(px(w), px(h)));
        }
        let images = self.images.clone();
        let protocol = self.protocol.clone();
        let clock = self.clock;
        let element = match &self.content {
            Content::World(world) => {
                let world = world.clone();
                canvas(
                    move |bounds, window, _| {
                        let mut world = world.borrow_mut();
                        let scene = world.frame(
                            bounds.size.width.into(),
                            bounds.size.height.into(),
                            clock.elapsed().as_secs_f64() * 1000.,
                            false,
                        );
                        // deka's world glue requests a frame unconditionally.
                        window.request_animation_frame();
                        scene
                    },
                    move |bounds, scene, window, _| {
                        paint_scene(bounds, &scene, window, &mut images.borrow_mut());
                        // Our work this frame: scene build (prepaint) + paint_scene.
                        let step = protocol.borrow_mut().end_frame(frame_start.elapsed());
                        if matches!(step, Step::Quit) || protocol.borrow_mut().idle_done() {
                            window.request_animation_frame();
                        }
                    },
                )
                .size_full()
                .into_any_element()
            }
            Content::Ui { host, renderer, scene } => {
                let root = host.borrow().render();
                let first = !self.protocol.borrow().first_frame_done();
                let renderer = renderer.clone();
                let scene_store = scene.clone();
                canvas(
                    move |bounds, window, _| {
                        let scene = renderer.render_at(
                            &root,
                            bounds.size.width.into(),
                            bounds.size.height.into(),
                            window.scale_factor(),
                            clock.elapsed().as_secs_f64() * 1000.,
                            false,
                        );
                        // deka's UI glue requests frames only while animating.
                        if scene.animating {
                            window.request_animation_frame();
                        }
                        *scene_store.borrow_mut() = scene.clone();
                        if first {
                            common::mark("first scene built + laid out (deka UI)");
                        }
                        scene
                    },
                    move |bounds, scene, window, _| {
                        paint_scene(bounds, &scene, window, &mut images.borrow_mut());
                        if first {
                            common::mark("first paint recorded (GPUI draws + presents after this)");
                        }
                        let step = protocol.borrow_mut().end_frame(frame_start.elapsed());
                        let p = protocol.borrow();
                        // Non-idle runs keep animating to measure frames; idle runs do not.
                        if p.args.idle_secs.is_none() || matches!(step, Step::Quit) {
                            window.request_animation_frame();
                        }
                    },
                )
                .size_full()
                .into_any_element()
            }
        };
        // A finished run asks for one more frame, in which it quits.
        let p = self.protocol.borrow();
        if p.args.first_frame && p.frame_index() >= 1 {
            window.request_animation_frame();
        }
        drop(p);
        div().size_full().child(element).into_any_element()
    }
}

pub fn run(args: Args) {
    if args.app == AppKind::Text {
        text_reference();
        return;
    }
    if matches!(args.app, AppKind::Canvas | AppKind::Editor) {
        eprintln!("--app {:?} is vello-backend only", args.app);
        return;
    }
    common::trace("main");
    let protocol = Rc::new(RefCell::new(Protocol::new(args.clone(), "gpui")));
    let idle = args.idle_secs;
    let application = gpui::Application::new();
    common::mark("gpui Application::new returned");
    application.run(move |cx: &mut App| {
        common::mark("gpui app launched (run callback)");
        let logical = size(px(WINDOW_W), px(WINDOW_H));
        let origin = match cx.primary_display() {
            Some(d) => {
                let db = d.bounds();
                point(
                    db.origin.x + db.size.width - logical.width - px(8.),
                    db.origin.y + db.size.height - logical.height - px(8.),
                )
            }
            None => point(px(0.), px(0.)),
        };
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin, size: logical })),
                titlebar: Some(TitlebarOptions { title: Some("deka A/B: GPUI".into()), ..Default::default() }),
                focus: false,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| {
                    let content = match args.app {
                        AppKind::Ui => {
                            let c = Content::Ui {
                                host: Rc::new(RefCell::new(Host::new(common::UiApp))),
                                renderer: Rc::new(deka_native_ui::scene::Renderer::new()),
                                scene: Rc::new(RefCell::new(Scene::default())),
                            };
                            common::mark("fonts loaded (deka renderer: fontdue, embedded font) + app host");
                            c
                        }
                        _ => {
                            let mut world = World::new();
                            world.start();
                            world.set_muted(true);
                            Content::World(Rc::new(RefCell::new(world)))
                        }
                    };
                    View {
                        content,
                        clock: Instant::now(),
                        images: Rc::new(RefCell::new(HashMap::new())),
                        protocol: protocol.clone(),
                        quitting: false,
                    }
                })
            },
        );
        common::mark("gpui window opened (incl. Metal renderer + pipelines)");
        match opened {
            Ok(handle) if common::timeline_mode() => {
                // Stay up until the window server shows the window (or 3 s),
                // so "window on screen" is measured for GPUI too.
                cx.spawn(async move |cx| {
                    let t0 = Instant::now();
                    while !common::ON_SCREEN.load(std::sync::atomic::Ordering::Relaxed) && t0.elapsed() < std::time::Duration::from_secs(3) {
                        cx.background_executor().timer(std::time::Duration::from_millis(2)).await;
                    }
                    // Leave time for frame 1's presented handler (if instrumented).
                    cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
                    common::print_timeline("gpui");
                    let _ = handle.update(cx, |_, _, cx| cx.quit());
                })
                .detach();
            }
            Ok(handle) => {
                if let Some(secs) = idle {
                    // Idle runs end on a timer, not on a frame.
                    let protocol = protocol.clone();
                    cx.spawn(async move |cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs_f64(secs + 0.5))
                            .await;
                        protocol.borrow_mut().idle_done();
                        let _ = handle.update(cx, |_, _, cx| cx.quit());
                    })
                    .detach();
                }
            }
            Err(e) => {
                eprintln!("open window: {e:#}");
                cx.quit();
            }
        }
    });
}

/// CoreText reference for the text sheet, drawn with the exact CGContext
/// settings GPUI 0.2.2's macOS text system uses (grayscale AA, subpixel
/// positioning, no quantization, no hinting, no LCD smoothing).
fn text_reference() {
    use core_foundation::attributed_string::CFMutableAttributedString;
    use core_foundation::base::{CFRange, TCFType};
    use core_foundation::string::CFString;
    use core_graphics::color_space::CGColorSpace;
    use core_graphics::context::{CGContext, CGTextDrawingMode};
    use core_graphics::data_provider::CGDataProvider;
    use core_graphics::font::CGFont;
    use core_text::font::new_from_CGFont;
    use core_text::line::CTLine;
    use core_text::string_attributes::kCTFontAttributeName;

    for scale in [1u32, 2] {
        let (w, h) = ((common::SHEET_W as u32) * scale, (common::SHEET_H as u32) * scale);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        let cx = CGContext::create_bitmap_context(
            Some(rgba.as_mut_ptr() as *mut _),
            w as usize,
            h as usize,
            8,
            w as usize * 4,
            &CGColorSpace::create_device_rgb(),
            core_graphics::base::kCGImageAlphaPremultipliedLast,
        );
        cx.scale(scale as f64, scale as f64);
        cx.set_text_drawing_mode(CGTextDrawingMode::CGTextFill);
        cx.set_rgb_fill_color(0x11 as f64 / 255., 0x11 as f64 / 255., 0x11 as f64 / 255., 1.0);
        cx.set_allows_antialiasing(true);
        cx.set_should_antialias(true);
        cx.set_allows_font_subpixel_positioning(true);
        cx.set_should_subpixel_position_fonts(true);
        cx.set_allows_font_subpixel_quantization(false);
        cx.set_should_subpixel_quantize_fonts(false);
        // SAFETY: FONT_BYTES is 'static, so the provider never outlives its data.
        let provider = unsafe { CGDataProvider::from_slice(common::FONT_BYTES) };
        let Ok(cg_font) = CGFont::from_data_provider(provider) else {
            eprintln!("CGFont from bundled bytes failed");
            return;
        };
        let mut y = 12.0f64;
        for size in common::TEXT_SIZES {
            let font = new_from_CGFont(&cg_font, size as f64);
            let line_text = format!("{size}px  {}", common::TEXT_SAMPLE);
            let mut attributed = CFMutableAttributedString::new();
            attributed.replace_str(&CFString::new(&line_text), CFRange::init(0, 0));
            let len = attributed.char_len();
            unsafe {
                attributed.set_attribute(CFRange::init(0, len), kCTFontAttributeName, &font);
            }
            let line = CTLine::new_with_attributed_string(attributed.as_concrete_TypeRef());
            let ascent = font.ascent();
            let descent = font.descent();
            let baseline = y + ascent;
            // CoreGraphics is y-up.
            cx.set_text_position(12.0, common::SHEET_H as f64 - baseline);
            line.draw(&cx);
            y += ascent + descent + 10.0;
        }
        drop(cx);
        common::write_png(&common::shots_dir().join(format!("text-coretext-{scale}x.png")), w, h, &rgba);
    }
    println!("[gpui] wrote CoreText reference sheets (1x, 2x)");
}
