use crate::{Application as NativeApplication, Host, scene::Scene};
use gpui::{prelude::*, *};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

struct View<A: NativeApplication> {
    host: Host<A>,
    clock: Instant,
    reduced_motion: bool,
    focus: FocusHandle,
    focused: Option<String>,
    renderer: Rc<crate::scene::Renderer>,
    scene: Rc<RefCell<Scene>>,
    images: Rc<RefCell<HashMap<String, Arc<RenderImage>>>>,
}
impl<A: NativeApplication> Render for View<A> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = self.host.render();
        let clock = self.clock;
        let reduced_motion = self.reduced_motion;
        let renderer = self.renderer.clone();
        let scene_store = self.scene.clone();
        let images = self.images.clone();
        let focused = self.focused.clone();
        let focus = self.focus.clone();
        div()
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus)
            .tab_index(0)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    window.focus(&focus);
                    let scene = view.scene.borrow();
                    if let Some(target) =
                        scene.hit(f32::from(event.position.x), f32::from(event.position.y))
                    {
                        view.focused = Some(target.id.clone());
                        view.host.click(target.handler);
                        cx.notify();
                    }
                }),
            )
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                let scene = view.scene.borrow();
                if event.keystroke.key == "tab" {
                    let current = scene
                        .targets
                        .iter()
                        .position(|t| Some(&t.id) == view.focused.as_ref());
                    let next = match (current, event.keystroke.modifiers.shift) {
                        (None, false) => Some(0),
                        (None, true) => scene.targets.len().checked_sub(1),
                        (Some(i), false) => Some(i + 1),
                        (Some(i), true) => i.checked_sub(1),
                    };
                    view.focused = next
                        .and_then(|i| scene.targets.get(i))
                        .map(|t| t.id.clone());
                    cx.notify();
                    if view.focused.is_some() {
                        cx.stop_propagation();
                    }
                } else if matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && let Some(target) = scene
                        .targets
                        .iter()
                        .find(|t| Some(&t.id) == view.focused.as_ref())
                {
                    view.host.click(target.handler);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                canvas(
                    move |bounds, window, _| {
                        let mut scene = renderer.render_at(
                            &root,
                            bounds.size.width.into(),
                            bounds.size.height.into(),
                            window.scale_factor(),
                            clock.elapsed().as_secs_f64() * 1000.,
                            reduced_motion,
                        );
                        if scene.animating {
                            window.request_animation_frame();
                        }
                        if let Some(id) = focused {
                            scene.focus_ring(&id);
                        }
                        *scene_store.borrow_mut() = scene.clone();
                        scene
                    },
                    move |bounds, scene, window, _| {
                        paint_scene(bounds, &scene, window, &mut images.borrow_mut());
                    },
                )
                .size_full(),
            )
    }
}
pub fn run<A: NativeApplication>(app: A) {
    // This mode exercises the same application logic without opening a window.
    let args: Vec<_> = std::env::args().collect();
    if let Some(index) = args.iter().position(|a| a == "--exercise") {
        let clicks = args
            .get(index + 1)
            .and_then(|s| s.parse().ok())
            .expect("--exercise requires a nonnegative click count");
        println!("{}", crate::exercise(app, clicks));
        return;
    }
    let live = app.live();
    let reduced_motion = args.iter().any(|arg| arg == "--reduced-motion");
    gpui::Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(560.), px(300.)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Deka native experiment".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    if live {
                        cx.spawn(async move |view, cx| {
                            loop {
                                cx.background_executor()
                                    .timer(Duration::from_millis(100))
                                    .await;
                                if view
                                    .update(cx, |view: &mut View<A>, cx| {
                                        if view.host.refresh() {
                                            cx.notify();
                                        }
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                    }
                    View {
                        host: Host::new(app),
                        clock: Instant::now(),
                        reduced_motion,
                        focus: cx.focus_handle(),
                        focused: None,
                        renderer: std::rc::Rc::new(crate::scene::Renderer::new()),
                        scene: std::rc::Rc::new(std::cell::RefCell::new(
                            crate::scene::Scene::default(),
                        )),
                        images: std::rc::Rc::new(std::cell::RefCell::new(HashMap::new())),
                    }
                })
            },
        )
        .expect("open native window");
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

/// Shared desktop painter for UI scenes and the portfolio-world example.
pub(crate) fn paint_scene(
    bounds: Bounds<Pixels>,
    scene: &Scene,
    window: &mut Window,
    cache: &mut HashMap<String, Arc<RenderImage>>,
) {
    window.paint_quad(fill(bounds, rgb(scene.background)));
    // Opacity is applied to cached glyph alpha because GPUI's canvas image API
    // takes opacity from private element state. Keep only this frame's variants.
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
                        let rgba = image::RgbaImage::from_raw(
                            glyph.width as u32,
                            glyph.height as u32,
                            bgra,
                        )
                        .expect("glyph dimensions");
                        Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]))
                    });
                    let _ = window.paint_image(rect, Corners::default(), image.clone(), 0, false);
                }
            } else {
                window.paint_quad(
                    fill(
                        rect,
                        rgba((paint.color << 8) | (paint.opacity * 255.).round() as u32),
                    )
                    .corner_radii(px(paint.radius)),
                );
            }
        });
    }
    cache.retain(|id, _| used.contains(id));
}
