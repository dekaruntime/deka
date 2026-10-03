//! The benchmark scene: a force-graph-like picture of circles and lines,
//! every element moving every frame.
//!
//! `shapes` = number of animated shapes. Half are nodes (each a filled circle
//! with a stroked outline), half are edges (a stroked line between two moving
//! nodes). So a 10,000-shape frame encodes 5,000 fills + 10,000 strokes.
//!
//! Layouts:
//! * `Force`: a real (simple) force simulation every frame: all-pairs
//!   repulsion (Fruchterman-Reingold, O(n^2) with a 4k cutoff), springs on edges, a pull to the
//!   centre, plus a small wobble so it never fully settles. Starts from random
//!   positions. This is the "real graph view" case: per-frame CPU physics.
//! * `Laid`: no physics. Nodes on a jittered grid (what a force layout
//!   converges to: short edges), each orbiting its spot. Pure render benchmark.
//! * `Hairball`: no physics. Nodes anywhere, edges join random far-apart nodes
//!   (long lines across the whole canvas). Stress test for vello's buffers.

// kurbo/peniko come from the including module's `gfx` (vello's re-exports or
// vello_common's), so sparse-strips binaries can use this file without linking
// compute vello. Only `encode`/`VelloPainter` need vello itself.
#[cfg(any(feature = "vello", feature = "vello-backend"))]
use super::gfx::Scene;
#[cfg(any(feature = "vello", feature = "vello-backend"))]
use super::gfx::kurbo::Affine;
#[cfg(any(feature = "vello", feature = "vello-backend"))]
use super::gfx::peniko::Fill;
use super::gfx::kurbo::{Cap, Circle, Line, Shape, Stroke};
use super::gfx::peniko::Color;

pub const BACKGROUND: Color = Color::from_rgba8(0x14, 0x17, 0x22, 0xff);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Layout {
    Force,
    Laid,
    Hairball,
}

struct Node {
    x: f64,
    y: f64,
    orbit: f64,
    phase: f64,
    speed: f64,
    radius: f64,
    color: Color,
}

pub struct Graph {
    layout: Layout,
    nodes: Vec<Node>,
    edges: Vec<(usize, usize)>,
    pos: Vec<(f64, f64)>,
    disp: Vec<(f64, f64)>,
    /// Ideal edge length (force layout), logical px.
    ideal: f64,
    width: f64,
    height: f64,
}

/// Small deterministic generator so every run draws the same picture.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

const PALETTE: [(u8, u8, u8); 6] = [
    (0x4c, 0xc9, 0xf0),
    (0xf7, 0x25, 0x85),
    (0xb5, 0x17, 0x9e),
    (0x72, 0x09, 0xb7),
    (0xf9, 0xc7, 0x4f),
    (0x90, 0xbe, 0x6d),
];

impl Graph {
    /// `width`/`height` are logical pixels.
    pub fn new(shapes: usize, width: f64, height: f64, layout: Layout) -> Self {
        let n = (shapes / 2).max(1);
        let edge_count = shapes - n;
        let mut rng = Lcg(0x5eed_0000 + shapes as u64);
        let cols = ((n as f64 * width / height).sqrt().ceil() as usize).max(1);
        let rows = n.div_ceil(cols);
        let cell = (width / cols as f64).min(height / rows as f64);
        let base_r = (cell * 0.28).clamp(2.0, 14.0);
        let nodes = (0..n)
            .map(|i| {
                let (r, g, b) = PALETTE[i % PALETTE.len()];
                let (x, y, orbit) = match layout {
                    Layout::Laid => (
                        (i % cols) as f64 * cell + cell * (0.5 + 0.3 * (rng.next() - 0.5)),
                        (i / cols) as f64 * cell + cell * (0.5 + 0.3 * (rng.next() - 0.5)),
                        cell * 0.25,
                    ),
                    Layout::Hairball | Layout::Force => (
                        20.0 + rng.next() * (width - 40.0),
                        20.0 + rng.next() * (height - 40.0),
                        4.0 + rng.next() * 24.0,
                    ),
                };
                Node {
                    x,
                    y,
                    orbit,
                    phase: rng.next() * std::f64::consts::TAU,
                    speed: 0.5 + rng.next() * 1.5,
                    radius: base_r * (0.7 + rng.next() * 0.6),
                    color: Color::from_rgba8(r, g, b, 0xff),
                }
            })
            .collect::<Vec<_>>();
        let edges = (0..edge_count)
            .map(|k| {
                let a = k % n;
                let b = match layout {
                    // right, down, down-right neighbour (wrapping at the edges).
                    Layout::Laid | Layout::Force => match k % 3 {
                        0 => a + 1,
                        1 => a + cols,
                        _ => a + cols + 1,
                    },
                    Layout::Hairball => a + 1 + (rng.next() * 40.0) as usize,
                };
                (a, b % n)
            })
            .collect();
        let pos = nodes.iter().map(|n| (n.x, n.y)).collect();
        Graph {
            layout,
            disp: vec![(0.0, 0.0); nodes.len()],
            pos,
            nodes,
            edges,
            ideal: cell * 0.8,
            width,
            height,
        }
    }

    /// Advance the picture to time `t` (seconds): one physics step for
    /// `Force`, closed-form orbits otherwise.
    pub fn update(&mut self, t: f64) {
        if self.layout != Layout::Force {
            for (p, n) in self.pos.iter_mut().zip(&self.nodes) {
                let a = n.phase + t * n.speed;
                *p = (n.x + n.orbit * a.cos(), n.y + n.orbit * (a * 1.3).sin());
            }
            return;
        }
        let k = self.ideal;
        let k2 = k * k;
        // Repulsion cutoff at 4k (still an O(n^2) distance test every frame).
        let cutoff2 = 16.0 * k2;
        let n = self.pos.len();
        for d in &mut self.disp {
            *d = (0.0, 0.0);
        }
        // All-pairs repulsion, k^2 / d, within the cutoff.
        for i in 0..n {
            let (xi, yi) = self.pos[i];
            let (mut fx, mut fy) = (0.0, 0.0);
            for j in (i + 1)..n {
                let (dx, dy) = (xi - self.pos[j].0, yi - self.pos[j].1);
                let d2 = (dx * dx + dy * dy).max(0.01);
                if d2 > cutoff2 {
                    continue;
                }
                let f = k2 / d2; // (k^2 / d) * (1 / d) to normalise the direction
                fx += dx * f;
                fy += dy * f;
                self.disp[j].0 -= dx * f;
                self.disp[j].1 -= dy * f;
            }
            self.disp[i].0 += fx;
            self.disp[i].1 += fy;
        }
        // Springs, d^2 / k.
        for &(a, b) in &self.edges {
            let (dx, dy) = (self.pos[a].0 - self.pos[b].0, self.pos[a].1 - self.pos[b].1);
            let d = (dx * dx + dy * dy).sqrt().max(0.1);
            let f = d / k; // (d^2 / k) * (1 / d)
            self.disp[a].0 -= dx * f;
            self.disp[a].1 -= dy * f;
            self.disp[b].0 += dx * f;
            self.disp[b].1 += dy * f;
        }
        // Gravity to the centre, a wobble so it never fully settles, and a
        // temperature cap on the step (FR "cooling" held at a constant).
        let (cx, cy) = (self.width / 2.0, self.height / 2.0);
        let temp = k * 0.1;
        for (i, node) in self.nodes.iter().enumerate() {
            let p = &mut self.pos[i];
            let a = node.phase + t * node.speed * 2.0;
            let (mut dx, mut dy) = self.disp[i];
            dx += 0.08 * (cx - p.0) + k * 0.05 * a.cos();
            dy += 0.08 * (cy - p.1) + k * 0.05 * a.sin();
            let len = (dx * dx + dy * dy).sqrt();
            if len > 0.0 {
                let s = len.min(temp) / len;
                p.0 = (p.0 + dx * s).clamp(10.0, self.width - 10.0);
                p.1 = (p.1 + dy * s).clamp(10.0, self.height - 10.0);
            }
        }
    }

    /// Encode the current positions into `scene` at device `scale`.
    #[cfg(any(feature = "vello", feature = "vello-backend"))]
    pub fn encode(&self, scene: &mut Scene, scale: f64) {
        scene.reset();
        self.paint(&mut VelloPainter { scene, xf: Affine::scale(scale) });
    }

    /// Emit the frame through any 2D API (vello, vello_cpu, vello_gpu share it).
    pub fn paint(&self, p: &mut impl Painter) {
        let edge_stroke = Stroke::new(1.0).with_caps(Cap::Round);
        let edge_color = Color::from_rgba8(0xc8, 0xd0, 0xe0, 0x60);
        for &(a, b) in &self.edges {
            p.stroke(&Line::new(self.pos[a], self.pos[b]), &edge_stroke, edge_color);
        }
        let outline = Stroke::new(1.0);
        let outline_color = Color::from_rgba8(0xff, 0xff, 0xff, 0xc0);
        for (pos, n) in self.pos.iter().zip(&self.nodes) {
            let c = Circle::new(*pos, n.radius);
            p.fill(&c, n.color);
            p.stroke(&c, &outline, outline_color);
        }
    }
}

/// The two calls the benchmark scene needs. Shapes are in logical px; the
/// painter applies the device scale.
pub trait Painter {
    fn fill(&mut self, shape: &impl Shape, color: Color);
    fn stroke(&mut self, shape: &impl Shape, style: &Stroke, color: Color);
}

#[cfg(any(feature = "vello", feature = "vello-backend"))]
struct VelloPainter<'a> {
    scene: &'a mut Scene,
    xf: Affine,
}

#[cfg(any(feature = "vello", feature = "vello-backend"))]
impl Painter for VelloPainter<'_> {
    fn fill(&mut self, shape: &impl Shape, color: Color) {
        self.scene.fill(Fill::NonZero, self.xf, color, None, shape);
    }
    fn stroke(&mut self, shape: &impl Shape, style: &Stroke, color: Color) {
        self.scene.stroke(style, self.xf, color, None, shape);
    }
}
