//! Portable layout, font rasterization, paint commands and hit testing.
//! Both platform adapters consume this scene; neither lays out application elements.
use crate::Node;
use fontdue::{Font, FontSettings};
use serde::Serialize;
use std::collections::HashMap;
use taffy::prelude::*;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
#[derive(Clone, Serialize)]
pub struct Paint {
    pub rect: Rect,
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
pub struct Target {
    pub id: String,
    pub handler: usize,
    pub rect: Rect,
}
#[derive(Default, Clone, Serialize)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub background: u32,
    pub paint: Vec<Paint>,
    pub images: Vec<GlyphImage>,
    pub targets: Vec<Target>,
}
impl Scene {
    pub fn hit(&self, x: f32, y: f32) -> Option<&Target> {
        if x < 0. || y < 0. || x >= self.width || y >= self.height {
            return None;
        }
        self.targets
            .iter()
            .rev()
            .find(|target| target.rect.contains(x, y))
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
                    color: 0xffc800,
                    radius: 0.,
                    image: None,
                });
            }
        }
    }
}
struct Item<'a> {
    node: &'a Node,
    layout: NodeId,
    color: u32,
    font_size: f32,
    children: Vec<Item<'a>>,
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
    fn text_width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|c| self.font.metrics(c, size).advance_width)
            .sum()
    }
    fn tree<'a>(
        &self,
        node: &'a Node,
        color: u32,
        font_size: f32,
        tree: &mut TaffyTree<Size<f32>>,
    ) -> Item<'a> {
        let s = &node.style;
        let color = s.color.unwrap_or(color);
        let font_size = s.font_size.unwrap_or(font_size);
        let children: Vec<_> = node
            .children
            .iter()
            .map(|n| self.tree(n, color, font_size, tree))
            .collect();
        let style = taffy::Style {
            display: Display::Flex,
            flex_direction: if s.row {
                FlexDirection::Row
            } else {
                FlexDirection::Column
            },
            size: Size {
                width: s.width.map_or(Dimension::auto(), Dimension::length),
                height: s.height.map_or(Dimension::auto(), Dimension::length),
            },
            padding: taffy::Rect::length(s.padding),
            gap: Size::length(s.gap),
            flex_shrink: 0.,
            ..Default::default()
        };
        let layout = if let Some(text) = &node.text {
            tree.new_leaf_with_context(
                style,
                Size {
                    width: self.text_width(text, font_size),
                    height: font_size * 1.4,
                },
            )
            .expect("text layout")
        } else {
            tree.new_with_children(
                style,
                &children.iter().map(|c| c.layout).collect::<Vec<_>>(),
            )
            .expect("element layout")
        };
        Item {
            node,
            layout,
            color,
            font_size,
            children,
        }
    }
    pub fn render(&self, root: &Node, width: f32, height: f32, scale: f32) -> Scene {
        let width = width.clamp(1., 8192.);
        let height = height.clamp(1., 8192.);
        let scale = scale.clamp(1., 3.);
        let mut tree = TaffyTree::new();
        let item = self.tree(root, 0x1a1611, 16., &mut tree);
        if root.style.width.is_none() {
            let mut style = tree.style(item.layout).expect("root style").clone();
            style.size.width = Dimension::length(width);
            tree.set_style(item.layout, style).expect("viewport width");
        }
        // The viewport constrains automatic widths; explicit widths remain explicit.
        tree.compute_layout_with_measure(
            item.layout,
            Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
            |known, _, _, context, _| {
                let measured = context.copied().unwrap_or(Size::ZERO);
                Size {
                    width: known.width.unwrap_or(measured.width),
                    height: known.height.unwrap_or(measured.height),
                }
            },
        )
        .expect("scene layout");
        let mut scene = Scene {
            width,
            height,
            background: root.style.background.unwrap_or(0xffffff),
            ..Default::default()
        };
        let mut images = HashMap::new();
        self.paint(&item, &tree, 0., 0., scale, &mut scene, &mut images);
        scene.images = images.into_values().collect();
        scene.images.sort_by(|a, b| a.id.cmp(&b.id));
        scene
    }
    #[allow(clippy::too_many_arguments)] // Recursive traversal carries one shared scene and its glyph table.
    fn paint(
        &self,
        item: &Item<'_>,
        tree: &TaffyTree<Size<f32>>,
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
        if let Some(color) = item.node.style.background {
            scene.paint.push(Paint {
                rect,
                color,
                radius: item.node.style.radius,
                image: None,
            });
        }
        if let Some(handler) = item.node.on_click {
            scene.targets.push(Target {
                id: item.node.id.clone(),
                handler,
                rect,
            });
        }
        if let Some(text) = &item.node.text {
            let font_px = (item.font_size * scale).round();
            let ascent = self
                .font
                .horizontal_line_metrics(font_px)
                .expect("font metrics")
                .ascent;
            let mut pen = rect.x + item.node.style.padding;
            for c in text.chars() {
                let (metrics, alpha) = self.font.rasterize(c, font_px);
                if metrics.width > 0 && metrics.height > 0 {
                    let id = format!("{}-{}-{}", c as u32, font_px, item.color);
                    images.entry(id.clone()).or_insert_with(|| {
                        let rgba = alpha
                            .iter()
                            .flat_map(|a| {
                                [
                                    (item.color >> 16) as u8,
                                    (item.color >> 8) as u8,
                                    item.color as u8,
                                    *a,
                                ]
                            })
                            .collect();
                        GlyphImage {
                            id: id.clone(),
                            width: metrics.width,
                            height: metrics.height,
                            rgba,
                        }
                    });
                    scene.paint.push(Paint {
                        rect: Rect {
                            x: pen + metrics.xmin as f32 / scale,
                            y: rect.y
                                + item.node.style.padding
                                + (ascent - metrics.height as f32 - metrics.ymin as f32) / scale,
                            width: metrics.width as f32 / scale,
                            height: metrics.height as f32 / scale,
                        },
                        color: item.color,
                        radius: 0.,
                        image: Some(id),
                    });
                }
                pen += self.font.metrics(c, item.font_size).advance_width;
            }
        }
        for child in &item.children {
            self.paint(child, tree, rect.x, rect.y, scale, scene, images);
        }
    }
}
