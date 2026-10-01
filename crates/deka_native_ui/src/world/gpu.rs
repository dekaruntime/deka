use super::{World, audio};
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
                        crate::gpu::paint_scene(bounds, &scene, window, &mut images.borrow_mut());
                    },
                )
                .size_full(),
            )
    }
}
pub fn run() {
    let reduced = std::env::args().any(|a| a == "--reduced-motion");
    let muted = std::env::args().any(|a| a == "--muted");
    gpui::Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Fernwood · Deka portfolio world · M to mute".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
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
