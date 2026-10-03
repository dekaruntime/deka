//! deka's window on GPUI 0.2.2: main's glue (crates/deka_native_ui/src/gpu.rs and
//! src/world/gpu.rs at the base commit), copied with only crate paths changed and
//! one hook after each paint. Not product code.
use native_backend_compare::*;

fn native_backend_compare_hook() {
    HOOK.with(|h| (h.borrow_mut())());
}
/// Bounds centred on the harness's virtual display, and that display, so the
/// window never opens on a physical screen (the only change to main's glue).
fn on_virtual_display(w: f32, h: f32, cx: &gpui::App) -> (gpui::Bounds<gpui::Pixels>, Option<gpui::DisplayId>) {
    let target = require_virtual_display();
    let Some(display) = cx.displays().into_iter().find(|d| u32::from(d.id()) == target) else {
        eprintln!("GPUI does not list display {target}");
        std::process::exit(2);
    };
    let id = display.id();
    (gpui::Bounds::centered(Some(id), gpui::size(gpui::px(w), gpui::px(h)), cx), Some(id))
}

thread_local! {
    static HOOK: std::cell::RefCell<Box<dyn FnMut()>> = std::cell::RefCell::new(Box::new(|| {}));
}

#[allow(dead_code, reason = "main's glue, copied as is")]
mod glue {
    use super::native_backend_compare_hook;
    use deka_native_ui::{Application as NativeApplication, Host, scene::Scene};
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
        renderer: Rc<deka_native_ui::scene::Renderer>,
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
                            native_backend_compare_hook();
                        },
                    )
                    .size_full(),
                )
        }
    }
    pub fn run<A: NativeApplication>(app: A, title: &str, w: f32, h: f32) {
        let title: gpui::SharedString = title.to_owned().into();
        // This mode exercises the same application logic without opening a window.
        let args: Vec<_> = std::env::args().collect();
        if let Some(index) = args.iter().position(|a| a == "--exercise") {
            let clicks = args
                .get(index + 1)
                .and_then(|s| s.parse().ok())
                .expect("--exercise requires a nonnegative click count");
            println!("{}", deka_native_ui::exercise(app, clicks));
            return;
        }
        let live = app.live();
        let reduced_motion = args.iter().any(|arg| arg == "--reduced-motion");
        gpui::Application::new().run(move |cx: &mut App| {
            let (bounds, display_id) = super::on_virtual_display(w, h, cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some(title.clone()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    display_id,
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
                            renderer: std::rc::Rc::new(deka_native_ui::scene::Renderer::new()),
                            scene: std::rc::Rc::new(std::cell::RefCell::new(
                                deka_native_ui::scene::Scene::default(),
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
    pub fn paint_scene(
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
}

#[allow(dead_code, reason = "main's glue, copied as is")]
mod world_glue {
    use super::native_backend_compare_hook;
    use deka_native_ui::world::{World, audio};
    use gpui::{prelude::*, *};
    use rodio::{Source, buffer::SamplesBuffer};
    use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc, time::Instant};

    struct Audio {
        _stream: rodio::OutputStream,
        music: rodio::Sink,
        effects: RefCell<Vec<rodio::Sink>>,
    }
    impl Audio {
        fn new(muted: bool) -> Result<Self, rodio::StreamError> {
            let stream = rodio::OutputStreamBuilder::open_default_stream()?;
            let music = rodio::Sink::connect_new(stream.mixer());
            if muted {
                music.pause();
            }
            music
                .append(SamplesBuffer::new(1, audio::SAMPLE_RATE, audio::samples(2)).repeat_infinite());
            Ok(Self {
                _stream: stream,
                music,
                effects: RefCell::new(vec![]),
            })
        }
        fn update(&self, world: &mut World, active: bool) {
            let muted = world.snapshot().muted || !active;
            let mut effects = self.effects.borrow_mut();
            effects.retain(|sink| !sink.empty());
            if muted {
                self.music.pause();
                for sink in effects.drain(..) {
                    sink.stop();
                }
            } else {
                self.music.play();
            }
            for id in world.drain_sounds() {
                if !muted {
                    let sink = rodio::Sink::connect_new(self._stream.mixer());
                    sink.append(SamplesBuffer::new(
                        1,
                        audio::SAMPLE_RATE,
                        audio::samples(id),
                    ));
                    effects.push(sink);
                }
            }
        }
    }
    struct WorldView {
        world: Rc<RefCell<World>>,
        clock: Instant,
        focus: FocusHandle,
        images: Rc<RefCell<HashMap<String, Arc<RenderImage>>>>,
        audio: Rc<Option<Audio>>,
        reduced: bool,
        _blur: Subscription,
    }
    impl Render for WorldView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let world = self.world.clone();
            let images = self.images.clone();
            let clock = self.clock;
            let audio = self.audio.clone();
            let reduced = self.reduced;
            div()
                .size_full()
                .track_focus(&self.focus)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, window, _| window.focus(&view.focus)),
                )
                .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "m" && !event.is_held {
                        let muted = view.world.borrow().snapshot().muted;
                        view.world.borrow_mut().set_muted(!muted);
                    } else {
                        view.world.borrow_mut().key(&event.keystroke.key, true);
                    }
                    cx.notify();
                }))
                .on_key_up(cx.listener(|view, event: &KeyUpEvent, _, cx| {
                    view.world.borrow_mut().key(&event.keystroke.key, false);
                    cx.notify();
                }))
                .child(
                    canvas(
                        move |bounds, window, _| {
                            let mut world = world.borrow_mut();
                            if !window.is_window_active() {
                                world.blur();
                            }
                            let scene = world.frame(
                                bounds.size.width.into(),
                                bounds.size.height.into(),
                                clock.elapsed().as_secs_f64() * 1000.,
                                reduced,
                            );
                            if let Some(audio) = audio.as_ref() {
                                audio.update(&mut world, window.is_window_active());
                            }
                            window.request_animation_frame();
                            scene
                        },
                        move |bounds, scene, window, _| {
                            super::glue::paint_scene(bounds, &scene, window, &mut images.borrow_mut());
                            native_backend_compare_hook();
                        },
                    )
                    .size_full(),
                )
        }
    }
    pub fn run() {
        let reduced = std::env::args().any(|a| a == "--reduced-motion");
        let muted = true;
        gpui::Application::new().run(move |cx: &mut App| {
            let (bounds, display_id) = super::on_virtual_display(960., 640., cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Fernwood · Deka portfolio world · M to mute".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    display_id,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let focus = cx.focus_handle();
                        window.focus(&focus);
                        let world = Rc::new(RefCell::new(World::new()));
                        world.borrow_mut().start();
                        world.borrow_mut().set_muted(muted);
                        let audio = match Audio::new(muted) {
                            Ok(a) => Some(a),
                            Err(e) => {
                                eprintln!("World audio unavailable: {e}");
                                None
                            }
                        };
                        let blur = cx.on_blur(&focus, window, |view: &mut WorldView, _, _| {
                            view.world.borrow_mut().blur()
                        });
                        WorldView {
                            world,
                            clock: Instant::now(),
                            focus,
                            images: Rc::new(RefCell::new(HashMap::new())),
                            audio: Rc::new(audio),
                            reduced,
                            _blur: blur,
                        }
                    })
                },
            )
            .expect("open portfolio world");
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    }
}

/// Paint one fixed scene and read the drawable back (patched GPUI, see Cargo.toml).
mod snap {
    use deka_native_ui::scene::Scene;
    use gpui::{prelude::*, *};
    use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};
    struct View {
        scene: Rc<Scene>,
        images: Rc<RefCell<HashMap<String, Arc<RenderImage>>>>,
        frames: Rc<RefCell<u32>>,
    }
    impl Render for View {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let scene = self.scene.clone();
            let images = self.images.clone();
            let frames = self.frames.clone();
            div().size_full().child(
                canvas(
                    |_, window, _| window.request_animation_frame(),
                    move |bounds, (), window, _| {
                        super::glue::paint_scene(bounds, &scene, window, &mut images.borrow_mut());
                        let mut n = frames.borrow_mut();
                        *n += 1;
                        // Read back each of the first frames; the last one read wins.
                        if *n <= 6 {
                            gpui::readback::REQUEST.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    },
                )
                .size_full(),
            )
        }
    }
    pub fn run(scene: Scene, out: std::path::PathBuf) {
        gpui::readback::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
        // GPUI draws frame one even before the window is visible; later frames
        // only while it is. Take the last frame read back within 1.5 s of the first.
        std::thread::spawn(move || {
            while gpui::readback::RESULT.lock().is_none() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
            if let Some((w, h, rgba)) = gpui::readback::RESULT.lock().take() {
                native_backend_compare::write_png(&out, w, h, &rgba);
                println!("{}: {w}x{h}", out.display());
            }
            std::process::exit(0);
        });
        let (w, h) = (scene.width, scene.height);
        let scene = Rc::new(scene);
        gpui::Application::new().run(move |cx: &mut App| {
            let (bounds, display_id) = super::on_virtual_display(w, h, cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    display_id,
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| View {
                        scene,
                        images: Default::default(),
                        frames: Default::default(),
                    })
                },
            )
            .expect("open window");
            cx.activate(true);
        });
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("snap") {
        let out = std::path::PathBuf::from(arg("--out").unwrap_or_else(|| "shots".into()));
        let name = arg("--scene").expect("--scene NAME");
        require_virtual_display();
        // A window that cannot be shown (locked screen) never paints; give up.
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(20));
            eprintln!("snap: no frame in 20 s (is the screen locked?)");
            std::process::exit(3);
        });
        let (scene, _) = scene(&name);
        snap::run(scene, out.join(format!("{name}-gpui.png")));
        return;
    }
    let protocol = Protocol::new("gpui");
    HOOK.with(|h| *h.borrow_mut() = Box::new(move || protocol.frame()));
    match app_name().as_str() {
        "world" => world_glue::run(),
        "settings" => glue::run(Settings, "Settings", 960., 640.),
        _ => glue::run(vm_app(COUNTER, "Counter"), "Deka native experiment", 560., 300.),
    }
}
