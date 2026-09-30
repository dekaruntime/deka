//! Portable layout, font rasterization, paint commands and hit testing.
//! Both platform adapters consume this scene; neither lays out application elements.
use crate::Node;
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
    pub id: String,
    pub rect: Rect,
    pub clip: Rect,
    pub text: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct Target {
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
}
impl Scene {
    pub fn hit(&self, x: f32, y: f32) -> Option<&Target> {
        if x < 0. || y < 0. || x >= self.width || y >= self.height {
            return None;
        }
        self.targets
            .iter()
            .rev()
            .find(|target| target.rect.contains(x, y) && target.clip.contains(x, y))
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
                });
            }
        }
    }
}
pub struct Renderer {
    font: Font,
}
impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}
impl Renderer {
    pub fn new() -> Self {
        Self {
            font: Font::from_bytes(
                include_bytes!("../assets/AtkinsonHyperlegible-Regular.ttf") as &[u8],
                FontSettings::default(),
            )
            .expect("bundled font"),
        }
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
        scene: &mut Scene,
        images: &mut HashMap<String, GlyphImage>,
    ) {
        let layout = tree.layout(item.layout).expect("computed layout");
        let rect = Rect {
            x: x + layout.location.x,
            y: y + layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        };
        scene.nodes.push(NodeBox {
            id: item.node.id.clone(),
            rect,
            clip,
            text: item.node.text.clone(),
        });
        if let Some(color) = item.node.style.background {
            scene.paint.push(Paint {
                rect,
                clip,
                color,
                radius: item.node.style.radius,
                image: None,
            });
        }
        if let Some(handler) = item.node.on_click
            && rect.intersection(clip).is_some()
        {
            scene.targets.push(Target {
                id: item.node.id.clone(),
                handler,
                rect,
                clip,
            });
        }
        let content_clip = if item.node.style.clip {
            rect.intersection(clip).unwrap_or_default()
        } else {
            clip
        };
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
                scene.paint.push(Paint {
                    rect: Rect {
                        x: rect.x + padding.left + g.x - logical.xmin as f32
                            + metrics.xmin as f32 / scale,
                        y: rect.y + padding.top + g.y + logical.height as f32 + logical.ymin as f32
                            - (metrics.height as f32 + metrics.ymin as f32) / scale,
                        width: metrics.width as f32 / scale,
                        height: metrics.height as f32 / scale,
                    },
                    clip: content_clip,
                    color: item.color,
                    radius: 0.,
                    image: Some(id),
                });
            }
        }
        for child in &item.children {
            self.paint(
                child,
                tree,
                content_clip,
                rect.x,
                rect.y,
                scale,
                scene,
                images,
            );
        }
    }
}
