//! Shared affine geometry and a portable transformed-paint fallback.
use crate::scene::{GlyphImage, Paint, Rect};
use serde::Serialize;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Transform(pub [f32; 6]);
impl Default for Transform {
    fn default() -> Self {
        Self([1., 0., 0., 1., 0., 0.])
    }
}
impl Transform {
    pub fn compose(self, next: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = next.0;
        Self([
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ])
    }
    pub fn translation(x: f32, y: f32) -> Self {
        Self([1., 0., 0., 1., x, y])
    }
    pub fn around(rect: Rect, scale: f32, degrees: f32) -> Self {
        let r = degrees.to_radians();
        let (s, c) = r.sin_cos();
        let x = rect.x + rect.width / 2.;
        let y = rect.y + rect.height / 2.;
        Self::translation(x, y)
            .compose(Self([c * scale, s * scale, -s * scale, c * scale, 0., 0.]))
            .compose(Self::translation(-x, -y))
    }
    pub fn point(self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }
    pub fn inverse(self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        (
            (d * (x - e) - c * (y - f)) / det,
            (-b * (x - e) + a * (y - f)) / det,
        )
    }
    pub fn bounds(self, r: Rect) -> Rect {
        let pts = [
            self.point(r.x, r.y),
            self.point(r.x + r.width, r.y),
            self.point(r.x, r.y + r.height),
            self.point(r.x + r.width, r.y + r.height),
        ];
        let x = pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let y = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        Rect {
            x,
            y,
            width: pts.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max) - x,
            height: pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max) - y,
        }
    }
    fn axis(self) -> bool {
        self.0[1].abs() < 0.000001 && self.0[2].abs() < 0.000001 && self.0[0] > 0. && self.0[3] > 0.
    }
}
#[derive(Clone, Serialize)]
pub struct Clip {
    pub rect: Rect,
    pub transform: Transform,
}
impl Clip {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let (x, y) = self.transform.inverse(x, y);
        self.rect.contains(x, y)
    }
}
/// Scale/translation use ordinary quads. Rotation uses shared pixels until both
/// adapters expose textured affine primitives. Work is bounded by the viewport.
pub(crate) fn paint(
    mut paint: Paint,
    transform: Transform,
    clips: &[Clip],
    scale: f32,
    images: &mut HashMap<String, GlyphImage>,
) -> Option<Paint> {
    let mut bounds = transform.bounds(paint.rect);
    let mut clip = paint.clip;
    for c in clips {
        clip = clip.intersection(c.transform.bounds(c.rect))?;
    }
    if transform.axis() && clips.iter().all(|c| c.transform.axis()) {
        paint.rect = bounds;
        paint.clip = clip;
        paint.radius *= transform.0[0];
        return Some(paint);
    }
    bounds = bounds.intersection(clip)?;
    // Limit a fallback tile to one megapixel; very large rotations trade resolution
    // for bounded allocations. Native affine GPU primitives can remove this fallback.
    let scale = scale.min((1_048_576. / (bounds.width * bounds.height)).sqrt());
    let width = (bounds.width * scale).ceil() as usize;
    let height = (bounds.height * scale).ceil() as usize;
    if width == 0 || height == 0 {
        return None;
    }
    // Bound each raster tile to the already-clamped viewport dimensions.
    let source = paint.image.as_ref().and_then(|id| images.get(id));
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let gx = bounds.x + (x as f32 + 0.5) / scale;
            let gy = bounds.y + (y as f32 + 0.5) / scale;
            if !clips.iter().all(|c| c.contains(gx, gy)) {
                continue;
            }
            let (lx, ly) = transform.inverse(gx, gy);
            if !paint.rect.contains(lx, ly) {
                continue;
            }
            let out = &mut rgba[(y * width + x) * 4..][..4];
            if let Some(image) = source {
                let u =
                    ((lx - paint.rect.x) / paint.rect.width * image.width as f32).floor() as usize;
                let v = ((ly - paint.rect.y) / paint.rect.height * image.height as f32).floor()
                    as usize;
                let pos = (v.min(image.height - 1) * image.width + u.min(image.width - 1)) * 4;
                out.copy_from_slice(&image.rgba[pos..pos + 4]);
            } else {
                let radius = paint
                    .radius
                    .min(paint.rect.width / 2.)
                    .min(paint.rect.height / 2.);
                let dx = (lx - (paint.rect.x + paint.rect.width / 2.)).abs()
                    - (paint.rect.width / 2. - radius);
                let dy = (ly - (paint.rect.y + paint.rect.height / 2.)).abs()
                    - (paint.rect.height / 2. - radius);
                // Viewport-bounded distances cannot overflow. Scalar f32 arithmetic
                // uses the same rounding on native and wasm; platform hypotf
                // differed by one alpha unit at a rounded corner at DPR 2.
                let outer_x = dx.max(0.);
                let outer_y = dy.max(0.);
                let distance =
                    (outer_x * outer_x + outer_y * outer_y).sqrt() + dx.max(dy).min(0.) - radius;
                let alpha = (0.5 - distance * scale).clamp(0., 1.);
                out.copy_from_slice(&[
                    (paint.color >> 16) as u8,
                    (paint.color >> 8) as u8,
                    paint.color as u8,
                    (alpha * 255.).round() as u8,
                ]);
            }
        }
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (width as u32).hash(&mut hash);
    (height as u32).hash(&mut hash);
    hash.write(&rgba);
    let id = format!("affine-{:x}", hash.finish());
    images.entry(id.clone()).or_insert(GlyphImage {
        id: id.clone(),
        width,
        height,
        rgba,
    });
    paint.rect = bounds;
    paint.clip = clip;
    paint.image = Some(id);
    paint.radius = 0.;
    Some(paint)
}
