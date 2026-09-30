//! Portable layout, font rasterization, paint commands and hit testing.
//! Both platform adapters consume this scene; neither lays out application elements.
use crate::{
    Node,
    geometry::{Clip, Transform},
};
use fontdue::{Font, FontSettings};
use serde::Serialize;
use std::collections::HashMap;
use taffy::prelude::*;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            width: (right - x).max(0.),
            height: (bottom - y).max(0.),
        })
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
#[derive(Clone, Serialize)]
pub struct Paint {
    pub rect: Rect,
    pub clip: Rect,
    pub color: u32,
    pub radius: f32,
    pub image: Option<String>,
    pub opacity: f32,
}
#[derive(Clone, Serialize)]
pub struct GlyphImage {
    pub id: String,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Serialize)]
pub struct NodeBox {
    pub layout_rect: Rect,
    pub id: String,
    pub rect: Rect,
    pub clip: Rect,
    pub text: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct Target {
    pub hit_shapes: Vec<Clip>,
    pub id: String,
    pub handler: usize,
    pub rect: Rect,
    pub clip: Rect,
}
#[derive(Default, Clone, Serialize)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub background: u32,
    pub paint: Vec<Paint>,
    pub images: Vec<GlyphImage>,
    pub targets: Vec<Target>,
    pub nodes: Vec<NodeBox>,
    pub animating: bool,
}
impl Scene {
    pub fn hit(&self, x: f32, y: f32) -> Option<&Target> {
        if x < 0. || y < 0. || x >= self.width || y >= self.height {
            return None;
        }
        self.targets.iter().rev().find(|target| {
            target.rect.contains(x, y)
                && target.clip.contains(x, y)
                && target.hit_shapes.iter().all(|c| c.contains(x, y))
        })
    }
    pub fn focus_ring(&mut self, id: &str) {
        if let Some(target) = self.targets.iter().find(|t| t.id == id) {
            let r = target.rect;
            for rect in [
                Rect {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: 2.,
                },
                Rect {
                    x: r.x,
                    y: r.y + r.height - 2.,
                    width: r.width,
                    height: 2.,
                },
                Rect {
                    x: r.x,
                    y: r.y,
                    width: 2.,
                    height: r.height,
                },
                Rect {
                    x: r.x + r.width - 2.,
                    y: r.y,
                    width: 2.,
                    height: r.height,
                },
            ] {
                self.paint.push(Paint {
                    rect,
                    clip: target.clip,
                    color: 0xffc800,
                    radius: 0.,
                    image: None,
                    opacity: 1.,
                });
            }
        }
    }
}
pub struct Renderer {
    font: Font,
    layout_motion: std::cell::RefCell<crate::layout_motion::LayoutMotion>,
    animator: std::cell::RefCell<crate::animation::Animator>,
}
impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}
impl Renderer {
    pub fn new() -> Self {
        Self {
            animator: Default::default(),
            layout_motion: Default::default(),
            font: Font::from_bytes(
                include_bytes!("../assets/AtkinsonHyperlegible-Regular.ttf") as &[u8],
                FontSettings::default(),
            )
            .expect("bundled font"),
        }
    }
    pub fn reset_animations(&self) {
        self.animator.borrow_mut().clear();
        self.layout_motion.borrow_mut().clear();
    }
    pub fn render_at(
        &self,
        root: &Node,
        width: f32,
        height: f32,
        scale: f32,
        milliseconds: f64,
        reduced_motion: bool,
    ) -> Scene {
        let (mut root, active) =
            self.animator
                .borrow_mut()
                .sample(root, milliseconds, reduced_motion);
        let mut scene = self.render(&root, width, height, scale);
        let layout_active =
            self.layout_motion
                .borrow_mut()
                .apply(&mut root, &scene, milliseconds, reduced_motion);
        if layout_active {
            scene = self.render(&root, width, height, scale);
        }
        scene.animating = active || layout_active;
        scene
    }
    pub fn render(&self, root: &Node, width: f32, height: f32, scale: f32) -> Scene {
        let width = width.clamp(1., 8192.);
        let height = height.clamp(1., 8192.);
        let scale = scale.clamp(1., 3.);
        let root = crate::layout::normalize(root);
        let mut tree = TaffyTree::new();
        let item = crate::layout::tree(&root, 0x1a1611, 16., false, &mut tree);
        if root.style.width == crate::Length::Auto {
            let mut style = tree.style(item.layout).expect("root style").clone();
            style.size.width = Dimension::length(width);
            tree.set_style(item.layout, style).expect("viewport width");
        }
        tree.compute_layout_with_measure(
            item.layout,
            Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
            |known, available, _, context, _| {
                crate::layout::measure(&self.font, known, available, context)
            },
        )
        .expect("scene layout");
        let mut scene = Scene {
            width,
            height,
            background: 0xffffff,
            ..Default::default()
        };
        let mut images = HashMap::new();
        self.paint(
            &item,
            &tree,
            Rect {
                x: 0.,
                y: 0.,
                width,
                height,
            },
            0.,
            0.,
            scale,
            1.,
            Transform::default(),
            &[],
            &mut scene,
            &mut images,
        );
        scene.images = images.into_values().collect();
        scene.images.sort_by(|a, b| a.id.cmp(&b.id));
        scene
    }
    #[allow(clippy::too_many_arguments)] // Recursive traversal carries one scene, glyph table and ancestor clip.
    fn paint(
        &self,
        item: &crate::layout::Item<'_>,
        tree: &TaffyTree<crate::layout::TextMeasure>,
        clip: Rect,
        x: f32,
        y: f32,
        scale: f32,
        opacity: f32,
        parent_transform: Transform,
        clips: &[Clip],
        scene: &mut Scene,
        images: &mut HashMap<String, GlyphImage>,
    ) {
        let layout = tree.layout(item.layout).expect("computed layout");
        let opacity = opacity * item.node.style.opacity.clamp(0., 1.);
        let rect = Rect {
            x: x + layout.location.x,
            y: y + layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        };
        let transform = parent_transform
            .compose(Transform::translation(
                item.node.style.translate_x,
                item.node.style.translate_y,
            ))
            .compose(Transform::around(
                rect,
                item.node.style.scale,
                item.node.style.rotate,
            ));
        let world = transform.bounds(rect);
        let mut effective_clip = clip;
        for shape in clips {
            effective_clip = effective_clip
                .intersection(shape.transform.bounds(shape.rect))
                .unwrap_or_default();
        }
        scene.nodes.push(NodeBox {
            layout_rect: rect,
            id: item.node.id.clone(),
            rect: world,
            clip: effective_clip,
            text: item.node.text.clone(),
        });
        if let Some(color) = item.node.style.background {
            let paint = Paint {
                rect,
                clip,
                color,
                radius: item.node.style.radius,
                image: None,
                opacity,
            };
            if let Some(paint) = crate::geometry::paint(paint, transform, clips, scale, images) {
                scene.paint.push(paint);
            }
        }
        if let Some(handler) = item.node.on_click
            && opacity > 0.
            && world.intersection(effective_clip).is_some()
        {
            scene.targets.push(Target {
                hit_shapes: {
                    let mut shapes = clips.to_vec();
                    shapes.push(Clip { rect, transform });
                    shapes
                },
                id: item.node.id.clone(),
                handler,
                rect: world,
                clip: effective_clip,
            });
        }
        let mut content_clips = clips.to_vec();
        if item.node.style.clip {
            content_clips.push(Clip { rect, transform });
        }
        if let Some(text) = &item.node.text {
            let padding = layout.padding;
            let width = (rect.width - padding.left - padding.right).max(0.);
            let text_layout = crate::text::layout(
                &self.font,
                text,
                item.font_size,
                (!item.nowrap).then_some(width),
            );
            for g in text_layout.glyphs() {
                let size = item.font_size * scale;
                let (metrics, alpha) = self.font.rasterize(g.parent, size);
                let logical = self.font.metrics(g.parent, item.font_size);
                if metrics.width == 0 || metrics.height == 0 {
                    continue;
                }
                let id = format!("{}-{}-{}", g.parent as u32, size, item.color);
                images.entry(id.clone()).or_insert_with(|| GlyphImage {
                    id: id.clone(),
                    width: metrics.width,
                    height: metrics.height,
                    rgba: alpha
                        .iter()
                        .flat_map(|a| {
                            [
                                (item.color >> 16) as u8,
                                (item.color >> 8) as u8,
                                item.color as u8,
                                *a,
                            ]
                        })
                        .collect(),
                });
                let paint = Paint {
                    rect: Rect {
                        x: rect.x + padding.left + g.x - logical.xmin as f32
                            + metrics.xmin as f32 / scale,
                        y: rect.y + padding.top + g.y + logical.height as f32 + logical.ymin as f32
                            - (metrics.height as f32 + metrics.ymin as f32) / scale,
                        width: metrics.width as f32 / scale,
                        height: metrics.height as f32 / scale,
                    },
                    clip,
                    color: item.color,
                    radius: 0.,
                    image: Some(id),
                    opacity,
                };
                if let Some(paint) =
                    crate::geometry::paint(paint, transform, &content_clips, scale, images)
                {
                    scene.paint.push(paint);
                }
            }
        }
        for child in &item.children {
            self.paint(
                child,
                tree,
                clip,
                rect.x,
                rect.y,
                scale,
                opacity,
                transform,
                &content_clips,
                scene,
                images,
            );
        }
    }
}
