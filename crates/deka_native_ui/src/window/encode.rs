//! deka's paint list -> vello_gpu drawing calls, and the image atlas bookkeeping.
//!
//! A deka scene is axis-aligned paints in logical pixels: rounded rectangles in
//! one colour and RGBA images (glyphs and transformed-paint tiles), each with an
//! opacity and a clip rectangle. Nothing here lays out or rasterises text.
use crate::scene::{GlyphImage, Paint, Scene};
use std::collections::{HashMap, HashSet};
use vello_gpu::kurbo::{Affine, Rect, RoundedRect};
use vello_gpu::peniko::{Color, ImageAlphaType, ImageBrush, ImageSampler};
use vello_gpu::{ImageId, ImageSource, PixelMetadata, Pixmap};

/// Images are uploaded in tiles no larger than this, so any image fits the
/// atlas (glyphs are one tile; a rotated element's fallback image may be several).
pub(crate) const MAX_TILE: u16 = 1024;

pub(crate) fn color(rgb: u32, alpha: f32) -> Color {
    Color::from_rgba8(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
        (alpha.clamp(0., 1.) * 255.).round() as u8,
    )
}

fn rect(r: crate::scene::Rect) -> Rect {
    Rect::new(
        f64::from(r.x),
        f64::from(r.y),
        f64::from(r.x + r.width),
        f64::from(r.y + r.height),
    )
}

/// One uploaded piece of an image, at `(x, y)` in the image's pixels.
#[derive(Clone, Debug)]
pub(crate) struct Tile {
    pub(crate) id: ImageId,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

/// Straight-alpha RGBA rows of `image` cut into atlas-sized pixmaps.
pub(crate) fn tiles(image: &GlyphImage) -> Vec<(u16, u16, Pixmap)> {
    let (w, h) = (image.width, image.height);
    if w == 0 || h == 0 || image.rgba.len() != w * h * 4 {
        return vec![];
    }
    let max = usize::from(MAX_TILE);
    let mut out = vec![];
    for y in (0..h).step_by(max) {
        for x in (0..w).step_by(max) {
            let (tw, th) = ((w - x).min(max), (h - y).min(max));
            let mut data = Vec::with_capacity(tw * th * 4);
            for row in y..y + th {
                let start = (row * w + x) * 4;
                data.extend_from_slice(&image.rgba[start..start + tw * 4]);
            }
            // Both fit u16: tiles are at most MAX_TILE, offsets below an image
            // dimension that the scene clamps to 8192 logical pixels at 3x.
            let (Ok(x), Ok(y), Ok(tw), Ok(th)) = (
                u16::try_from(x),
                u16::try_from(y),
                u16::try_from(tw),
                u16::try_from(th),
            ) else {
                return vec![];
            };
            let pixmap = Pixmap::from_parts(
                data,
                tw,
                th,
                PixelMetadata {
                    may_have_transparency: true,
                    alpha_type: ImageAlphaType::Alpha,
                },
            );
            out.push((x, y, pixmap));
        }
    }
    out
}

/// The images of the current scene that live in the renderer's atlas.
#[derive(Default)]
pub(crate) struct Images {
    uploaded: HashMap<String, Vec<Tile>>,
}

impl Images {
    /// Ids the scene draws.
    pub(crate) fn used(scene: &Scene) -> HashSet<&str> {
        scene
            .paint
            .iter()
            .filter(|p| p.opacity > 0.)
            .filter_map(|p| p.image.as_deref())
            .collect()
    }

    /// Images to upload (drawn but not in the atlas) and ids to evict (in the
    /// atlas but no longer drawn). The previous frame's images stay resident.
    pub(crate) fn plan<'a>(&self, scene: &'a Scene) -> (Vec<&'a GlyphImage>, Vec<String>) {
        let used = Self::used(scene);
        let missing = scene
            .images
            .iter()
            .filter(|g| used.contains(g.id.as_str()) && !self.uploaded.contains_key(&g.id))
            .collect();
        let stale = self
            .uploaded
            .keys()
            .filter(|id| !used.contains(id.as_str()))
            .cloned()
            .collect();
        (missing, stale)
    }

    pub(crate) fn insert(&mut self, id: String, tiles: Vec<Tile>) {
        self.uploaded.insert(id, tiles);
    }

    pub(crate) fn remove(&mut self, id: &str) -> Vec<Tile> {
        self.uploaded.remove(id).unwrap_or_default()
    }

    pub(crate) fn get(&self, id: &str) -> Option<&[Tile]> {
        self.uploaded.get(id).map(Vec::as_slice)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.uploaded.len()
    }
}

/// Record `scene` into `target`, scaled to device pixels.
///
/// Clips are applied as rectangle clips and shared by consecutive paints with
/// the same clip; a paint entirely inside its clip needs none. Images are
/// snapped to the device pixel grid (glyph rasters are already device-sized).
pub(crate) fn encode(target: &mut vello_gpu::Scene, scene: &Scene, scale: f64, images: &Images) {
    let s = Affine::scale(scale);
    target.set_transform(s);
    target.reset_paint_transform();
    target.set_paint(color(scene.background, 1.));
    target.fill_rect(&Rect::new(
        0.,
        0.,
        f64::from(scene.width),
        f64::from(scene.height),
    ));
    let mut active: Option<Rect> = None;
    for paint in &scene.paint {
        if paint.opacity <= 0. {
            continue;
        }
        let r = rect(paint.rect);
        let c = rect(paint.clip);
        let inside = |outer: Rect| {
            r.x0 >= outer.x0 && r.y0 >= outer.y0 && r.x1 <= outer.x1 && r.y1 <= outer.y1
        };
        if !inside(c) && active != Some(c) {
            if active.is_some() {
                target.pop_clip();
            }
            target.set_transform(s);
            target.push_clip_rect(&c);
            active = Some(c);
        } else if inside(c)
            && let Some(a) = active
            && !inside(a)
        {
            target.pop_clip();
            active = None;
        }
        match &paint.image {
            Some(id) => draw_image(target, paint, r, scale, images.get(id)),
            None => {
                target.set_transform(s);
                target.set_paint(color(paint.color, paint.opacity));
                if paint.radius > 0. {
                    target.fill_shape(&RoundedRect::from_rect(r, f64::from(paint.radius)));
                } else {
                    target.fill_rect(&r);
                }
            }
        }
    }
    if active.is_some() {
        target.pop_clip();
    }
}

fn draw_image(
    target: &mut vello_gpu::Scene,
    paint: &Paint,
    r: Rect,
    scale: f64,
    tiles: Option<&[Tile]>,
) {
    let Some(tiles) = tiles else { return };
    let (image_w, image_h) = tiles.iter().fold((0u32, 0u32), |(w, h), t| {
        (
            w.max(u32::from(t.x) + u32::from(t.width)),
            h.max(u32::from(t.y) + u32::from(t.height)),
        )
    });
    if image_w == 0 || image_h == 0 {
        return;
    }
    let (x, y) = ((r.x0 * scale).round(), (r.y0 * scale).round());
    let (sx, sy) = (
        r.width() * scale / f64::from(image_w),
        r.height() * scale / f64::from(image_h),
    );
    target.set_transform(Affine::IDENTITY);
    for tile in tiles {
        let (tx, ty) = (x + f64::from(tile.x) * sx, y + f64::from(tile.y) * sy);
        let brush = ImageBrush {
            image: ImageSource::opaque_id_with_transparency_hint(tile.id, true),
            sampler: ImageSampler::default(),
        }
        .with_alpha(paint.opacity);
        target.set_paint(brush);
        target.set_paint_transform(Affine::translate((tx, ty)) * Affine::scale_non_uniform(sx, sy));
        target.fill_rect(&Rect::new(
            tx,
            ty,
            tx + f64::from(tile.width) * sx,
            ty + f64::from(tile.height) * sy,
        ));
    }
    target.reset_paint_transform();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(id: &str, width: usize, height: usize) -> GlyphImage {
        GlyphImage {
            id: id.into(),
            width,
            height,
            rgba: (0..width * height)
                .flat_map(|i| [(i % 251) as u8, 0, 0, 255])
                .collect(),
        }
    }

    #[test]
    fn large_images_are_cut_into_atlas_sized_tiles_without_losing_pixels() {
        let big = image("big", 2500, 1100);
        let cut = tiles(&big);
        assert_eq!(cut.len(), 3 * 2);
        let covered: usize = cut
            .iter()
            .map(|(_, _, p)| usize::from(p.width()) * usize::from(p.height()))
            .sum();
        assert_eq!(covered, 2500 * 1100);
        assert!(
            cut.iter()
                .all(|(_, _, p)| p.width() <= MAX_TILE && p.height() <= MAX_TILE)
        );
        // The last tile starts where the earlier ones end, with the right pixels.
        let (x, y, last) = &cut[5];
        assert_eq!((*x, *y), (2048, 1024));
        let first = last.data_as_u8_slice()[0];
        assert_eq!(first, ((1024 * 2500 + 2048) % 251) as u8);
        assert!(tiles(&image("empty", 0, 3)).is_empty());
        let mut broken = image("short", 2, 2);
        broken.rgba.pop();
        assert!(tiles(&broken).is_empty(), "a malformed image is skipped");
    }

    #[test]
    fn uploads_only_new_images_and_evicts_ones_no_longer_drawn() {
        let paint = |id: &str| Paint {
            rect: Default::default(),
            clip: Default::default(),
            color: 0,
            radius: 0.,
            image: Some(id.into()),
            opacity: 1.,
        };
        let mut scene = Scene {
            images: vec![image("a", 1, 1), image("b", 1, 1)],
            paint: vec![paint("a"), paint("b")],
            ..Default::default()
        };
        let mut images = Images::default();
        let (missing, stale) = images.plan(&scene);
        assert_eq!(missing.len(), 2);
        assert!(stale.is_empty());
        for g in missing {
            images.insert(g.id.clone(), vec![]);
        }
        assert_eq!(images.plan(&scene).0.len(), 0, "nothing is uploaded twice");
        scene.paint.pop();
        scene.paint[0].opacity = 1.;
        let (missing, stale) = images.plan(&scene);
        assert!(missing.is_empty());
        assert_eq!(stale, vec!["b".to_owned()]);
        images.remove("b");
        assert_eq!(images.len(), 1);
    }
}
