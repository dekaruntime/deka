//! Portfolio-town experiment. Simulation, artwork, scene and PCM are platform-independent.
//! This is deliberately an example, not a new DekaScript or game-engine API.
mod art;
pub mod audio;
#[cfg(feature = "world-audio")]
mod window;
#[cfg(feature = "world-audio")]
pub use window::{run, run_with, world_options};

use crate::{
    Length, Node, Style,
    scene::{Paint, Rect, Renderer, Scene},
};
use serde::Serialize;

const STEP: f64 = 1. / 120.;
const SPEED: f32 = 100.;
const VIEW_W: f32 = 480.;
const VIEW_H: f32 = 320.;
const MAP_W: f32 = 960.;
const MAP_H: f32 = 640.;

pub struct Project {
    pub name: &'static str,
    pub subtitle: &'static str,
    pub description: &'static str,
    pub x: f32,
    pub y: f32,
    pub color: u32,
}
pub const PROJECTS: [Project; 3] = [
    Project {
        name: "Deka workshop",
        subtitle: "A language. A runtime. A place to build.",
        description: "Write expressive components and bring them to life. This town is a small experiment in the same Rust renderer that draws Deka's native interface.",
        x: 180.,
        y: 160.,
        color: 0xbc6261,
    },
    Project {
        name: "Atlas observatory",
        subtitle: "Ideas worth exploring.",
        description: "A placeholder for your next project: its story, screenshots and a hands-on demonstration. Each house can have its own interior and personality.",
        x: 420.,
        y: 120.,
        color: 0x587ba0,
    },
    Project {
        name: "Little greenhouse",
        subtitle: "Room for something new.",
        description: "A new project becomes a new place to visit. Add another building to the town's project data and give your idea a home.",
        x: 660.,
        y: 160.,
        color: 0x668657,
    },
];
const TREES: &[(f32, f32)] = &[
    (92., 190.),
    (114., 400.),
    (344., 212.),
    (590., 190.),
    (824., 270.),
    (390., 442.),
    (690., 435.),
    (58., 78.),
    (140., 72.),
    (790., 72.),
    (868., 100.),
    (830., 466.),
];

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Snapshot {
    pub x: f32,
    pub y: f32,
    pub room: Option<usize>,
    pub transitioning: bool,
    pub walking: bool,
    pub started: bool,
    pub muted: bool,
    pub hint: String,
}
#[derive(Clone, Copy)]
struct Transition {
    to: Option<usize>,
    elapsed: f32,
    switched: bool,
}

pub struct World {
    x: f32,
    y: f32,
    room: Option<usize>,
    held: [bool; 4],
    action_held: bool,
    facing: usize,
    steps: f32,
    walking: bool,
    started: bool,
    muted: bool,
    last: Option<f64>,
    accumulator: f64,
    transition: Option<Transition>,
    sounds: Vec<u8>,
    renderer: Renderer,
    sprites: Vec<crate::scene::GlyphImage>,
}
impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
impl World {
    pub fn new() -> Self {
        Self {
            x: 244.,
            y: 350.,
            room: None,
            held: [false; 4],
            action_held: false,
            facing: 0,
            steps: 0.,
            walking: false,
            started: false,
            muted: false,
            last: None,
            accumulator: 0.,
            transition: None,
            sounds: vec![],
            renderer: Renderer::new(),
            sprites: art::sprites(),
        }
    }
    pub fn start(&mut self) {
        self.started = true;
        self.blur();
    }
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.sounds.clear();
    }
    /// Hosts must call this on focus loss, visibility changes and suspended rendering.
    pub fn blur(&mut self) {
        self.held = [false; 4];
        self.action_held = false;
        self.walking = false;
        self.last = None;
        self.accumulator = 0.;
    }
    pub fn key(&mut self, key: &str, down: bool) -> bool {
        let direction = match key {
            "ArrowUp" | "up" | "w" | "W" => Some(0),
            "ArrowDown" | "down" | "s" | "S" => Some(1),
            "ArrowLeft" | "left" | "a" | "A" => Some(2),
            "ArrowRight" | "right" | "d" | "D" => Some(3),
            _ => None,
        };
        if let Some(i) = direction {
            self.held[i] = down;
            return true;
        }
        if matches!(
            key,
            "Enter" | "enter" | " " | "space" | "e" | "E" | "Escape" | "escape"
        ) {
            if down && !self.action_held {
                self.interact();
            }
            self.action_held = down;
            return true;
        }
        false
    }
    fn nearby(&self) -> Option<usize> {
        PROJECTS
            .iter()
            .position(|p| (self.x - p.x - 64.).abs() < 27. && (self.y - p.y - 139.).abs() < 26.)
    }
    pub fn interact(&mut self) {
        if !self.started {
            self.start();
            return;
        }
        if self.transition.is_some() {
            return;
        }
        if self.room.is_some() || self.nearby().is_some() {
            self.transition = Some(Transition {
                to: if self.room.is_some() {
                    None
                } else {
                    self.nearby()
                },
                elapsed: 0.,
                switched: false,
            });
            self.walking = false;
            self.held = [false; 4];
            self.sound(1);
        }
    }
    fn sound(&mut self, id: u8) {
        if !self.muted && self.sounds.len() < 8 {
            self.sounds.push(id);
        }
    }
    pub fn drain_sounds(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.sounds)
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            x: self.x,
            y: self.y,
            room: self.room,
            transitioning: self.transition.is_some(),
            walking: self.walking,
            started: self.started,
            muted: self.muted,
            hint: if let Some(i) = self.room {
                format!("{}  /  Enter to return to town", PROJECTS[i].name)
            } else if let Some(i) = self.nearby() {
                format!("Enter  /  Visit {}", PROJECTS[i].name)
            } else {
                "WASD or arrow keys to explore  /  Enter at a door".into()
            },
        }
    }
    /// Fixed 120 Hz simulation. Long gaps are dropped rather than teleporting the player.
    pub fn advance(&mut self, milliseconds: f64, reduced_motion: bool) {
        if !milliseconds.is_finite() {
            return;
        }
        let previous = self.last.unwrap_or(milliseconds);
        let now = milliseconds.max(previous);
        self.last = Some(now);
        if !self.started {
            return;
        }
        self.accumulator += ((now - previous) / 1000.).min(0.1);
        while self.accumulator + 1e-9 >= STEP {
            self.tick(STEP as f32, reduced_motion);
            self.accumulator -= STEP;
        }
    }
    fn tick(&mut self, dt: f32, reduced: bool) {
        if let Some(mut t) = self.transition {
            t.elapsed += dt;
            if (t.elapsed >= 0.22 || reduced) && !t.switched {
                let old = self.room;
                self.room = t.to;
                if let Some(i) = old {
                    self.x = PROJECTS[i].x + 64.;
                    self.y = PROJECTS[i].y + 158.;
                } else {
                    self.x = 240.;
                    self.y = 248.;
                }
                t.switched = true;
            }
            self.transition = if t.elapsed >= 0.44 || reduced {
                None
            } else {
                Some(t)
            };
            return;
        }
        let mut dx = i32::from(self.held[3]) as f32 - i32::from(self.held[2]) as f32;
        let mut dy = i32::from(self.held[1]) as f32 - i32::from(self.held[0]) as f32;
        if dx != 0. && dy != 0. {
            dx *= std::f32::consts::FRAC_1_SQRT_2;
            dy *= std::f32::consts::FRAC_1_SQRT_2;
        }
        if dy != 0. {
            self.facing = if dy < 0. { 1 } else { 0 };
        } else if dx != 0. {
            self.facing = if dx < 0. { 2 } else { 3 };
        }
        let before = (self.x, self.y);
        let x = self.x + dx * SPEED * dt;
        if self.free(x, self.y) {
            self.x = x;
        }
        let y = self.y + dy * SPEED * dt;
        if self.free(self.x, y) {
            self.y = y;
        }
        self.walking = before != (self.x, self.y);
        if self.walking {
            let step = (self.steps / 0.28) as u32;
            self.steps += dt;
            if (self.steps / 0.28) as u32 != step {
                self.sound(0);
            }
        }
        if self.room.is_some() && self.y > 275. && (self.x - 240.).abs() < 24. {
            self.interact();
        }
    }
    fn free(&self, x: f32, y: f32) -> bool {
        if self.room.is_some() {
            return (65. ..415.).contains(&x) && (167. ..284.).contains(&y);
        }
        if !(32. ..928.).contains(&x) || !(55. ..602.).contains(&y) {
            return false;
        }
        let intersects =
            |r: Rect| x + 7. > r.x && x - 7. < r.x + r.width && y > r.y && y - 7. < r.y + r.height;
        !PROJECTS.iter().any(|p| {
            intersects(Rect {
                x: p.x + 8.,
                y: p.y + 75.,
                width: 112.,
                height: 53.,
            })
        }) && !TREES.iter().any(|&(tx, ty)| {
            intersects(Rect {
                x: tx + 24.,
                y: ty + 63.,
                width: 16.,
                height: 21.,
            })
        }) && !intersects(Rect {
            x: 465.,
            y: 433.,
            width: 143.,
            height: 90.,
        })
    }
    pub fn frame(&mut self, width: f32, height: f32, milliseconds: f64, reduced: bool) -> Scene {
        self.advance(milliseconds, reduced);
        let mut scene = self.draw(reduced);
        let width = width.clamp(1., 8192.);
        let height = height.clamp(1., 8192.);
        let zoom = (width / VIEW_W).min(height / VIEW_H);
        let offset = ((width - VIEW_W * zoom) / 2., (height - VIEW_H * zoom) / 2.);
        for paint in &mut scene.paint {
            paint.rect.x = offset.0 + paint.rect.x * zoom;
            paint.rect.y = offset.1 + paint.rect.y * zoom;
            paint.rect.width *= zoom;
            paint.rect.height *= zoom;
            paint.radius *= zoom;
            paint.clip = Rect {
                x: offset.0,
                y: offset.1,
                width: VIEW_W * zoom,
                height: VIEW_H * zoom,
            };
        }
        scene.width = width;
        scene.height = height;
        scene.animating = self.started;
        scene
    }
    fn draw(&self, reduced: bool) -> Scene {
        let mut s = Scene {
            width: VIEW_W,
            height: VIEW_H,
            background: 0x172d30,
            ..Default::default()
        };
        if let Some(i) = self.room {
            self.interior(&mut s, i, reduced);
        } else {
            self.town(&mut s, reduced);
        }
        let hint = self.snapshot().hint;
        panel(&mut s, 10., 286., 460., 24., 0x203c39);
        self.text(&mut s, &hint, (21., 292.), 440., 11., 0xf9edcf);
        if let Some(t) = self.transition {
            let opacity = if t.elapsed < 0.22 {
                t.elapsed / 0.22
            } else {
                1. - (t.elapsed - 0.22) / 0.22
            };
            s.paint.push(Paint {
                rect: rect(0., 0., VIEW_W, VIEW_H),
                clip: rect(0., 0., VIEW_W, VIEW_H),
                color: 0x142b30,
                radius: 0.,
                image: None,
                opacity: opacity.clamp(0., 1.),
            });
        }
        // Only ship visible immutable sprite textures; the two GPU adapters cache by id.
        let used: std::collections::HashSet<_> =
            s.paint.iter().filter_map(|p| p.image.clone()).collect();
        s.images.extend(
            self.sprites
                .iter()
                .filter(|g| used.contains(&g.id))
                .cloned(),
        );
        s
    }
    fn town(&self, s: &mut Scene, reduced: bool) {
        let cx = (self.x - VIEW_W / 2.).clamp(0., MAP_W - VIEW_W).floor();
        let cy = (self.y - VIEW_H * 0.625).clamp(0., MAP_H - VIEW_H).floor();
        panel(s, 0., 0., VIEW_W, VIEW_H, 0x8bb878);
        for ty in 0..20 {
            for tx in 0..30 {
                let x = tx as f32 * 32. - cx;
                let y = ty as f32 * 32. - cy;
                if x > -32. && x < VIEW_W && y > -32. && y < VIEW_H {
                    sprite(
                        s,
                        if (tx * 7 + ty * 11) % 5 == 0 {
                            "flowers"
                        } else {
                            "grass"
                        },
                        x,
                        y,
                        32.,
                        32.,
                    );
                }
            }
        }
        panel(s, 90. - cx, 316. - cy, 780., 42., 0xc7b38a);
        panel(s, 90. - cx, 319. - cy, 780., 34., 0xe1ce9d);
        for p in PROJECTS {
            panel(
                s,
                p.x + 47. - cx,
                p.y + 128. - cy,
                34.,
                320. - p.y - 128.,
                0xe1ce9d,
            );
        }
        // The pond and its banks stay below all objects, with one immutable water sprite.
        sprite(s, "pond", 451. - cx, 420. - cy, 172., 116.);
        for tx in (40..920).step_by(24) {
            sprite(s, "fence", tx as f32 - cx, 40. - cy, 24., 20.);
            sprite(s, "fence", tx as f32 - cx, 595. - cy, 24., 20.);
        }
        let mut objects: Vec<(f32, usize)> = PROJECTS
            .iter()
            .enumerate()
            .map(|(i, p)| (p.y + 128., i))
            .collect();
        objects.extend(
            TREES
                .iter()
                .enumerate()
                .map(|(i, &(_, y))| (y + 82., i + 3)),
        );
        objects.push((self.y, 100));
        objects.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, id) in objects {
            if id == 100 {
                self.hero(s, self.x - cx, self.y - cy, reduced);
            } else if id < 3 {
                let p = &PROJECTS[id];
                sprite(s, &format!("house-{id}"), p.x - cx, p.y - cy, 128., 128.);
                sprite(s, "sign", p.x + 104. - cx, p.y + 123. - cy, 24., 24.);
            } else {
                let (x, y) = TREES[id - 3];
                sprite(s, "tree", x - cx, y - cy, 64., 96.);
            }
        }
        panel(s, 10., 10., 147., 34., 0x203c39);
        self.text(s, "FERNWOOD", (20., 15.), 130., 12., 0xffefd0);
        self.text(
            s,
            "a little world of things I build",
            (20., 30.),
            140.,
            8.,
            0xb5c9ab,
        );
        if let Some(i) = self.nearby() {
            let p = &PROJECTS[i];
            panel(s, p.x + 5. - cx, p.y - 18. - cy, 118., 19., 0x203c39);
            self.text(
                s,
                p.name,
                (p.x + 12. - cx, p.y - 15. - cy),
                110.,
                10.,
                0xffefd0,
            );
        }
    }
    fn hero(&self, s: &mut Scene, x: f32, y: f32, reduced: bool) {
        let frame = usize::from(self.walking && !reduced && (self.steps / 0.14) as u32 % 2 == 1);
        sprite(
            s,
            &format!("hero-{}-{frame}", self.facing),
            x.floor() - 12.,
            y.floor() - 32.,
            24.,
            36.,
        );
    }
    fn interior(&self, s: &mut Scene, i: usize, reduced: bool) {
        panel(s, 0., 0., 480., 320., 0x233b3e);
        panel(s, 48., 35., 384., 245., 0x594735);
        panel(s, 56., 43., 368., 72., 0xded6b6);
        panel(s, 56., 108., 368., 8., 0xbda77e);
        panel(s, 56., 116., 368., 163., 0xb79365);
        for y in (120..279).step_by(16) {
            panel(s, 56., y as f32, 368., 1., 0x9c7954);
        }
        for x in (64..416).step_by(48) {
            for y in (122..278).step_by(32) {
                panel(s, x as f32, y as f32, 1., 14., 0xa88258);
            }
        }
        panel(s, 171., 196., 138., 65., PROJECTS[i].color);
        panel(s, 176., 201., 128., 55., 0xd9bc8f);
        panel(s, 218., 273., 44., 7., 0x2b4140);
        sprite(s, "tree", 57., 91., 48., 72.);
        sprite(s, "tree", 373., 91., 48., 72.);
        self.hero(s, self.x, self.y, reduced);
        panel(s, 85., 52., 310., 133., 0x273e3c);
        self.text(s, PROJECTS[i].name, (100., 64.), 280., 18., 0xffdc99);
        self.text(s, PROJECTS[i].subtitle, (100., 89.), 280., 11., 0xc4d4b4);
        self.text(
            s,
            PROJECTS[i].description,
            (100., 111.),
            278.,
            12.,
            0xf5eddc,
        );
    }
    fn text(
        &self,
        s: &mut Scene,
        text: &str,
        position: (f32, f32),
        width: f32,
        size: f32,
        color: u32,
    ) {
        let node = Node {
            id: "world-label".into(),
            style: Style {
                width: Length::Px(width),
                font_size: Some(size),
                color: Some(color),
                ..Default::default()
            },
            text: Some(text.into()),
            on_click: None,
            children: vec![],
        };
        let label = self.renderer.render(&node, width, 200., 2.);
        for mut p in label.paint {
            p.rect.x += position.0;
            p.rect.y += position.1;
            s.paint.push(p);
        }
        for image in label.images {
            if !s.images.iter().any(|i| i.id == image.id) {
                s.images.push(image);
            }
        }
    }
}
fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}
fn panel(s: &mut Scene, x: f32, y: f32, w: f32, h: f32, color: u32) {
    s.paint.push(Paint {
        rect: rect(x, y, w, h),
        clip: rect(0., 0., VIEW_W, VIEW_H),
        color,
        radius: 0.,
        image: None,
        opacity: 1.,
    });
}
fn sprite(s: &mut Scene, id: &str, x: f32, y: f32, w: f32, h: f32) {
    if x + w <= 0. || y + h <= 0. || x >= VIEW_W || y >= VIEW_H {
        return;
    }
    s.paint.push(Paint {
        rect: rect(x, y, w, h),
        clip: rect(0., 0., VIEW_W, VIEW_H),
        color: 0xffffff,
        radius: 0.,
        image: Some(format!("world-{id}")),
        opacity: 1.,
    });
}

#[cfg(test)]
mod tests;
