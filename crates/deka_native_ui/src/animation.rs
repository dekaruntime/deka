//! Retained style targets. Hosts supply monotonic milliseconds; no platform clock lives here.
use crate::{Length, Node, Style};
use std::collections::{HashMap, HashSet};

struct Track {
    from: Style,
    target: Style,
    started: f64,
}
impl Track {
    fn sample(&self, now: f64) -> (Style, bool) {
        let progress = if self.target.duration_ms > 0. {
            ((now - self.started) / self.target.duration_ms as f64).clamp(0., 1.) as f32
        } else {
            1.
        };
        if progress >= 1. {
            return (self.target.clone(), false);
        }
        let t = match self.target.easing {
            1 => 1. - (1. - progress).powi(3),
            2 => progress * progress * (3. - 2. * progress),
            _ => progress,
        };
        let mut style = self.target.clone();
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let mask = self.target.transition;
        if mask & 1 != 0 {
            style.opacity = mix(self.from.opacity, style.opacity);
        }
        if mask & 2 != 0 {
            style.translate_x = mix(self.from.translate_x, style.translate_x);
            style.translate_y = mix(self.from.translate_y, style.translate_y);
        }
        if mask & 4 != 0 {
            let length = |a, b| match (a, b) {
                (Length::Px(a), Length::Px(b)) => Length::Px(mix(a, b)),
                (Length::Percent(a), Length::Percent(b)) => Length::Percent(mix(a, b)),
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
                    .round() as u32)
                        << shift)
                })),
                (_, b) => b,
            };
            style.background = color(self.from.background, style.background);
            style.color = color(self.from.color, style.color);
        }
        let active = progress < 1. && style != self.target;
        (style, active)
    }
}
#[derive(Default)]
pub struct Animator {
    tracks: HashMap<String, Track>,
    time: f64,
}
impl Animator {
    pub fn clear(&mut self) {
        self.tracks.clear();
    }
    pub fn sample(&mut self, root: &Node, milliseconds: f64, reduced_motion: bool) -> (Node, bool) {
        if milliseconds.is_finite() {
            self.time = self.time.max(milliseconds);
        }
        let mut seen = HashSet::new();
        let mut active = false;
        let node = self.node(root, reduced_motion, &mut seen, &mut active);
        self.tracks.retain(|id, _| seen.contains(id));
        (node, active)
    }
    fn node(
        &mut self,
        node: &Node,
        reduced: bool,
        seen: &mut HashSet<String>,
        active: &mut bool,
    ) -> Node {
        seen.insert(node.id.clone());
        let entry = self.tracks.entry(node.id.clone()).or_insert_with(|| Track {
            from: node.style.clone(),
            target: node.style.clone(),
            started: self.time,
        });
        if reduced {
            entry.from = node.style.clone();
            entry.target = node.style.clone();
            entry.started = self.time;
        } else if entry.target != node.style {
            entry.from = entry.sample(self.time).0;
            entry.target = node.style.clone();
            entry.started = self.time;
        }
        let (style, moving) = entry.sample(self.time);
        *active |= moving;
        Node {
            style,
            children: node
                .children
                .iter()
                .map(|n| self.node(n, reduced, seen, active))
                .collect(),
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
