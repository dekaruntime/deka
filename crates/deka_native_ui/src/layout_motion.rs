//! FLIP-style positional motion: layout supplies destinations, paint supplies offsets.
use crate::{Node, Style, scene::Scene};
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub(crate) struct LayoutMotion {
    tracks: HashMap<String, Track>,
    time: f64,
}
struct Track {
    target: (f32, f32),
    from: (f32, f32),
    started: f64,
    style: Style,
}
impl Track {
    fn position(&self, now: f64) -> ((f32, f32), bool) {
        let (t, done) = crate::motion::progress(&self.style, now - self.started);
        (
            (
                self.from.0 + (self.target.0 - self.from.0) * t,
                self.from.1 + (self.target.1 - self.from.1) * t,
            ),
            !done && self.from != self.target,
        )
    }
}
impl LayoutMotion {
    pub fn clear(&mut self) {
        self.tracks.clear();
    }
    pub fn apply(&mut self, root: &mut Node, scene: &Scene, now: f64, reduced: bool) -> bool {
        if now.is_finite() {
            self.time = self.time.max(now);
        }
        let positions: HashMap<_, _> = scene
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), (n.layout_rect.x, n.layout_rect.y)))
            .collect();
        let mut seen = HashSet::new();
        let mut active = false;
        self.node(root, &positions, (0., 0.), reduced, &mut seen, &mut active);
        self.tracks.retain(|id, _| seen.contains(id));
        active
    }
    #[allow(clippy::too_many_arguments)] // Traversal shares layout positions and the activity accumulator.
    fn node(
        &mut self,
        node: &mut Node,
        positions: &HashMap<&str, (f32, f32)>,
        inherited: (f32, f32),
        reduced: bool,
        seen: &mut HashSet<String>,
        active: &mut bool,
    ) {
        let mut offset = inherited;
        if node.style.motion.layout
            && let Some(&target) = positions.get(node.id.as_str())
        {
            seen.insert(node.id.clone());
            let track = self.tracks.entry(node.id.clone()).or_insert_with(|| Track {
                target,
                from: target,
                started: self.time,
                style: node.style.clone(),
            });
            if reduced {
                track.from = target;
                track.target = target;
            } else if track.target != target {
                track.from = track.position(self.time).0;
                track.target = target;
                track.started = self.time + node.style.motion.delay_ms as f64;
                track.style = node.style.clone();
            }
            let (position, moving) = track.position(self.time);
            *active |= moving;
            offset = (position.0 - target.0, position.1 - target.1);
            node.style.translate_x += offset.0 - inherited.0;
            node.style.translate_y += offset.1 - inherited.1;
        }
        for child in &mut node.children {
            self.node(child, positions, offset, reduced, seen, active);
        }
    }
}
