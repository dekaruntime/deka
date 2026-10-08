//! The portfolio world in a desktop window, with its music and effects.
use super::{World, audio};
use crate::scene::Scene;
use crate::window::{Content, Input, Options};
use rodio::{Source, buffer::SamplesBuffer};
use std::time::Instant;

struct Audio {
    stream: rodio::OutputStream,
    music: rodio::Sink,
    effects: Vec<rodio::Sink>,
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
            stream,
            music,
            effects: vec![],
        })
    }
    fn update(&mut self, world: &mut World, active: bool) {
        let muted = world.snapshot().muted || !active;
        self.effects.retain(|sink| !sink.empty());
        if muted {
            self.music.pause();
            for sink in self.effects.drain(..) {
                sink.stop();
            }
        } else {
            self.music.play();
        }
        for id in world.drain_sounds() {
            if !muted {
                let sink = rodio::Sink::connect_new(self.stream.mixer());
                sink.append(SamplesBuffer::new(
                    1,
                    audio::SAMPLE_RATE,
                    audio::samples(id),
                ));
                self.effects.push(sink);
            }
        }
    }
}

pub(crate) struct WorldContent {
    world: World,
    clock: Instant,
    reduced: bool,
    active: bool,
    audio: Option<Audio>,
    scene: Scene,
}

impl WorldContent {
    pub(crate) fn new(reduced: bool, muted: bool, audio: bool) -> Self {
        let mut world = World::new();
        world.start();
        world.set_muted(muted);
        let audio = audio
            .then(|| match Audio::new(muted) {
                Ok(audio) => Some(audio),
                Err(e) => {
                    eprintln!("World audio unavailable: {e}");
                    None
                }
            })
            .flatten();
        Self {
            world,
            clock: Instant::now(),
            reduced,
            active: true,
            audio,
            scene: Scene::default(),
        }
    }
}

impl Content for WorldContent {
    fn frame(&mut self, width: f32, height: f32, _scale: f32) -> &Scene {
        if !self.active {
            self.world.blur();
        }
        self.scene = self.world.frame(
            width,
            height,
            self.clock.elapsed().as_secs_f64() * 1000.,
            self.reduced,
        );
        &self.scene
    }

    fn input(&mut self, input: Input) -> bool {
        let input = match input {
            Input::EditKey(k) => Input::Key {
                name: k.name,
                down: k.down,
                repeat: false,
                shift: k.shift,
            },
            input => input,
        };
        match input {
            Input::Key {
                name,
                down: true,
                repeat,
                ..
            } if name == "m" => {
                if !repeat {
                    let muted = self.world.snapshot().muted;
                    self.world.set_muted(!muted);
                }
                true
            }
            Input::Key { name, down, .. } => {
                self.world.key(&name, down);
                true
            }
            Input::Focus(active) => {
                self.active = active;
                if !active {
                    self.world.blur();
                }
                true
            }
            Input::Press { .. }
            | Input::ContextMenu { .. }
            | Input::EditKey(_)
            | Input::Text(_)
            | Input::Preedit(..)
            | Input::Move { .. }
            | Input::Release => false,
        }
    }

    fn presented(&mut self, active: bool) {
        if let Some(audio) = self.audio.as_mut() {
            audio.update(&mut self.world, active);
        }
    }

    #[cfg(test)]
    fn scene(&self) -> &Scene {
        &self.scene
    }
}

/// Open the world in a window: `--reduced-motion`, `--muted`.
pub fn run() {
    let reduced = std::env::args().any(|a| a == "--reduced-motion");
    let muted = std::env::args().any(|a| a == "--muted");
    run_with(world_options(), reduced, muted);
}

/// The world's window.
pub fn world_options() -> Options {
    let mut options = Options::new("Fernwood · Deka portfolio world · M to mute", 960., 640.);
    options.background = 0x172d30;
    options
}

/// [`run`] with explicit window options (tests and measurements).
pub fn run_with(options: Options, reduced: bool, muted: bool) {
    let gpu = crate::window::start_gpu();
    crate::window::show(WorldContent::new(reduced, muted, true), options, gpu);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, down: bool, repeat: bool) -> Input {
        Input::Key {
            name: name.into(),
            down,
            repeat,
            shift: false,
        }
    }

    #[test]
    fn m_toggles_mute_once_per_press_and_keys_walk() {
        let mut content = WorldContent::new(true, false, false);
        assert!(content.input(key("m", true, false)));
        assert!(content.world.snapshot().muted);
        assert!(content.input(key("m", true, true)), "a held key repeats");
        assert!(
            content.world.snapshot().muted,
            "repeat does not toggle back"
        );
        content.input(key("m", false, false));
        content.input(key("m", true, false));
        assert!(!content.world.snapshot().muted);
        let start = content.world.snapshot().x;
        content.input(key("right", true, false));
        content.frame(960., 640., 2.);
        std::thread::sleep(std::time::Duration::from_millis(60));
        content.frame(960., 640., 2.);
        content.input(key("right", false, false));
        assert!(
            content.world.snapshot().x > start,
            "holding right walks right"
        );
        assert!(content.scene().animating);
    }

    #[test]
    fn losing_focus_releases_held_keys() {
        let mut content = WorldContent::new(true, true, false);
        content.input(key("right", true, false));
        assert!(content.input(Input::Focus(false)));
        content.frame(960., 640., 2.);
        let x = content.world.snapshot().x;
        std::thread::sleep(std::time::Duration::from_millis(40));
        content.frame(960., 640., 2.);
        assert_eq!(content.world.snapshot().x, x, "no walking without focus");
        assert!(!content.world.snapshot().walking);
    }
}
