//! Retained style targets. Hosts supply monotonic milliseconds; no platform clock lives here.
use crate::{Length, Node, Style};
use std::collections::{HashMap, HashSet};

struct Track {
    from: Style,
    target: Style,
    started: f64,
    motion_started: f64,
    exiting: bool,
}
impl Track {
    fn sample(&self, now: f64) -> (Style, bool) {
        let (t, done) = crate::motion::progress(&self.target, now - self.started);
        if done {
            return (self.target.clone(), false);
        }
        let mut style = self.target.clone();
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let mask = self.target.transition;
        if mask & 1 != 0 {
            style.opacity = mix(self.from.opacity, style.opacity);
        }
        if mask & 2 != 0 {
            style.translate_x = mix(self.from.translate_x, style.translate_x);
            style.translate_y = mix(self.from.translate_y, style.translate_y);
            style.scale = mix(self.from.scale, style.scale).max(0.01);
            style.rotate = mix(self.from.rotate, style.rotate);
        }
        if mask & 4 != 0 {
            let length = |a, b| match (a, b) {
                (Length::Px(a), Length::Px(b)) => Length::Px(mix(a, b).max(0.)),
                (Length::Percent(a), Length::Percent(b)) => Length::Percent(mix(a, b).max(0.)),
                (_, b) => b,
            };
            style.width = length(self.from.width, style.width);
            style.height = length(self.from.height, style.height);
        }
        if mask & 8 != 0 {
            let color = |a, b| match (a, b) {
                (Some(a), Some(b)) => Some([16, 8, 0].into_iter().fold(0, |rgb, shift| {
                    rgb | ((mix(
                        ((a >> shift) & 255u32) as f32,
                        ((b >> shift) & 255u32) as f32,
                    )
                    .clamp(0., 255.)
                    .round() as u32)
                        << shift)
                })),
                (_, b) => b,
            };
            style.background = color(self.from.background, style.background);
            style.color = color(self.from.color, style.color);
        }
        style.opacity = style.opacity.clamp(0., 1.);
        let active = (mask & 1 != 0 && self.from.opacity != self.target.opacity)
            || (mask & 2 != 0
                && (self.from.translate_x != self.target.translate_x
                    || self.from.translate_y != self.target.translate_y
                    || self.from.scale != self.target.scale
                    || self.from.rotate != self.target.rotate))
            || (mask & 4 != 0
                && (self.from.width != self.target.width
                    || self.from.height != self.target.height))
            || (mask & 8 != 0
                && (self.from.background != self.target.background
                    || self.from.color != self.target.color));
        (style, active)
    }
}
#[derive(Default)]
pub struct Animator {
    tracks: HashMap<String, Track>,
    previous: Option<Node>,
    time: f64,
}
impl Animator {
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.previous = None;
    }
    pub fn sample(&mut self, root: &Node, milliseconds: f64, reduced_motion: bool) -> (Node, bool) {
        if milliseconds.is_finite() {
            self.time = self.time.max(milliseconds);
        }
        let mut seen = HashSet::new();
        let mut active = false;
        let previous = self.previous.take();
        let node = self.node(
            root,
            previous.as_ref(),
            reduced_motion,
            0.,
            &mut seen,
            &mut active,
        );
        self.tracks.retain(|id, _| seen.contains(id));
        self.previous = Some(node.clone());
        (node, active)
    }
    fn ghost(&mut self, old: &Node, seen: &mut HashSet<String>, active: &mut bool) -> Option<Node> {
        let entry = self.tracks.get_mut(&old.id)?;
        if !entry.exiting {
            entry.from = entry.sample(self.time).0;
            entry.target = crate::motion::presence(&entry.target, old.style.motion.exit);
            entry.target.transition |= 3;
            entry.target.motion.frames.clear();
            entry.started = self.time + entry.target.motion.delay_ms as f64;
            entry.exiting = true;
        }
        let (style, moving) = entry.sample(self.time);
        if !moving {
            return None;
        }
        seen.insert(old.id.clone());
        *active = true;
        let mut ghost = old.clone();
        ghost.style = style;
        fn inert(node: &mut Node) {
            node.on_click = None;
            for child in &mut node.children {
                inert(child);
            }
        }
        inert(&mut ghost);
        Some(ghost)
    }
    #[allow(clippy::too_many_arguments)] // Recursive presence reconciliation carries the previous tree and one activity accumulator.
    fn node(
        &mut self,
        node: &Node,
        old: Option<&Node>,
        reduced: bool,
        stagger: f64,
        seen: &mut HashSet<String>,
        active: &mut bool,
    ) -> Node {
        seen.insert(node.id.clone());
        let entry = self.tracks.entry(node.id.clone()).or_insert_with(|| {
            let mut from = node.style.clone();
            let mut target = node.style.clone();
            if !reduced && target.motion.enter != 0 {
                from = crate::motion::presence(&target, target.motion.enter);
                target.transition |= 3;
            }
            Track {
                from,
                target,
                started: self.time + node.style.motion.delay_ms as f64 + stagger,
                motion_started: self.time + node.style.motion.delay_ms as f64 + stagger,
                exiting: false,
            }
        });
        let mut target = node.style.clone();
        if target.motion.enter != 0 {
            target.transition |= 3;
        }
        if reduced {
            entry.from = target.clone();
            entry.target = target;
            entry.started = self.time;
            entry.exiting = false;
        } else if entry.target != target || entry.exiting {
            entry.from = entry.sample(self.time).0;
            if entry.target.motion.frames != target.motion.frames
                || entry.target.motion.repeats != target.motion.repeats
                || entry.target.duration_ms != target.duration_ms
            {
                entry.motion_started = self.time + target.motion.delay_ms as f64 + stagger;
            }
            entry.target = target;
            entry.started = self.time + node.style.motion.delay_ms as f64 + stagger;
            entry.exiting = false;
        }
        let (mut style, moving) = entry.sample(self.time);
        *active |= moving;
        if !reduced {
            *active |= crate::motion::keyframes(&mut style, self.time - entry.motion_started);
        }
        let mut children: Vec<Node> = node
            .children
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let previous = old.and_then(|p| p.children.iter().find(|c| c.id == n.id));
                self.node(
                    n,
                    previous,
                    reduced,
                    i as f64 * node.style.motion.stagger_ms as f64,
                    seen,
                    active,
                )
            })
            .collect();
        if !reduced && let Some(old) = old {
            for (i, child) in old.children.iter().enumerate() {
                if !node.children.iter().any(|n| n.id == child.id)
                    && child.style.motion.exit != 0
                    && let Some(ghost) = self.ghost(child, seen, active)
                {
                    children.insert(i.min(children.len()), ghost);
                }
            }
        }
        Node {
            style,
            children,
            ..node.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: &str, opacity: f32) -> Node {
        Node {
            id: id.into(),
            style: Style {
                opacity,
                transition: 1,
                duration_ms: 1000.,
                easing: 0,
                ..Default::default()
            },
            text: None,
            on_click: None,
            children: vec![],
        }
    }
    #[test]
    fn identity_survives_reordering_and_removal_releases_tracks() {
        let mut animation = Animator::default();
        let mut root = node("root", 1.);
        root.children = vec![node("a", 0.), node("b", 1.)];
        animation.sample(&root, 0., false);
        root.children[0].style.opacity = 1.;
        animation.sample(&root, 10., false);
        root.children.swap(0, 1);
        let (half, _) = animation.sample(&root, 510., false);
        assert_eq!(half.children[1].id, "a");
        assert_eq!(half.children[1].style.opacity, 0.5);
        root.children.clear();
        animation.sample(&root, 520., false);
        assert_eq!(animation.tracks.len(), 1);
        root.children.push(node("a", 1.));
        assert!(
            !animation.sample(&root, 530., false).1,
            "remount must not revive an old transition"
        );
    }
    #[test]
    fn easing_zero_duration_and_clock_rollback_are_explicit() {
        let mut animation = Animator::default();
        let mut target = node("a", 0.);
        target.style.easing = 1;
        animation.sample(&target, 0., false);
        target.style.opacity = 1.;
        animation.sample(&target, 0., false);
        assert_eq!(
            animation.sample(&target, 500., false).0.style.opacity,
            0.875
        );
        assert_eq!(
            animation.sample(&target, 250., false).0.style.opacity,
            0.875
        );
        target.style.duration_ms = 0.;
        target.style.opacity = 0.;
        let (end, active) = animation.sample(&target, 600., false);
        assert_eq!(end.style.opacity, 0.);
        assert!(!active);
    }
}
