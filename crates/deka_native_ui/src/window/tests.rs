//! What a broken window would get wrong: the pixels of the drawing path, and
//! the scene a resize or a scale change produces. GPU tests need an adapter; CI
//! runs them on macOS (`desktop-build`), where every runner has Metal.
use super::*;

#[cfg(target_os = "macos")]
mod gpu {
    use super::*;
    use crate::scene::{GlyphImage, Paint, Rect};

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn fill(r: Rect, clip: Rect, color: u32, opacity: f32, radius: f32) -> Paint {
        Paint {
            rect: r,
            clip,
            color,
            radius,
            image: None,
            opacity,
        }
    }

    const FULL: Rect = Rect {
        x: 0.,
        y: 0.,
        width: 100.,
        height: 80.,
    };

    /// A scene exercising every kind of paint deka emits.
    fn sample() -> Scene {
        let glyph = GlyphImage {
            id: "g".into(),
            width: 8,
            height: 8,
            // Left half opaque green, right half transparent.
            rgba: (0..64)
                .flat_map(|i| {
                    if i % 8 < 4 {
                        [0, 200, 0, 255]
                    } else {
                        [0, 200, 0, 0]
                    }
                })
                .collect(),
        };
        Scene {
            width: 100.,
            height: 80.,
            background: 0xffffff,
            paint: vec![
                // An opaque red square.
                fill(rect(10., 10., 20., 20.), FULL, 0xff0000, 1., 0.),
                // A blue square at half opacity.
                fill(rect(40., 10., 20., 20.), FULL, 0x0000ff, 0.5, 0.),
                // A black bar clipped to its left half.
                fill(
                    rect(10., 40., 40., 10.),
                    rect(10., 40., 20., 10.),
                    0,
                    1.,
                    0.,
                ),
                // A round black disc (corner pixels stay white).
                fill(rect(60., 40., 20., 20.), FULL, 0, 1., 10.),
                // An invisible paint draws nothing.
                fill(rect(0., 60., 100., 20.), FULL, 0, 0., 0.),
                // A glyph image at device scale (4 logical px at 2x = 8 device px).
                Paint {
                    rect: rect(85., 10., 4., 4.),
                    clip: FULL,
                    color: 0,
                    radius: 0.,
                    image: Some("g".into()),
                    opacity: 1.,
                },
            ],
            images: vec![glyph],
            ..Default::default()
        }
    }

    fn close(actual: [u8; 4], expected: [u8; 4]) -> bool {
        actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2)
    }

    #[track_caller]
    fn assert_pixel(shot: &Snapshot, x: u32, y: u32, expected: [u8; 4]) {
        let actual = shot.pixel(x, y);
        assert!(
            close(actual, expected),
            "pixel ({x}, {y}) is {actual:?}, expected {expected:?}"
        );
    }

    #[test]
    fn draws_every_kind_of_paint_at_device_scale() {
        let shot = snapshot(&sample(), 2.).expect("offscreen render");
        assert_eq!((shot.width, shot.height), (200, 160));
        const WHITE: [u8; 4] = [255, 255, 255, 255];
        assert_pixel(&shot, 2, 2, WHITE);
        // Red is red: channel order survives the BGRA surface format.
        assert_pixel(&shot, 40, 40, [255, 0, 0, 255]);
        // Half-opaque blue over white.
        assert_pixel(&shot, 100, 40, [127, 127, 255, 255]);
        // Clipped: the bar's left half is drawn, its right half is not.
        assert_pixel(&shot, 30, 90, [0, 0, 0, 255]);
        assert_pixel(&shot, 80, 90, WHITE);
        // The disc's centre is filled; its bounding box's corner is not.
        assert_pixel(&shot, 140, 100, [0, 0, 0, 255]);
        assert_pixel(&shot, 121, 81, WHITE);
        // The zero-opacity paint left the bottom band white.
        assert_pixel(&shot, 100, 150, WHITE);
        // The glyph: opaque left half, transparent right half, 1:1 pixels.
        assert_pixel(&shot, 171, 22, [0, 200, 0, 255]);
        assert_pixel(&shot, 176, 22, WHITE);
    }

    #[test]
    fn a_scale_change_renders_the_same_scene_at_the_new_density() {
        let one = snapshot(&sample(), 1.).expect("1x");
        let two = snapshot(&sample(), 2.).expect("2x");
        assert_eq!((one.width * 2, one.height * 2), (two.width, two.height));
        for (x, y) in [(20, 20), (50, 20), (15, 45), (70, 50)] {
            assert!(
                close(one.pixel(x, y), two.pixel(x * 2, y * 2)),
                "({x}, {y}) differs between 1x and 2x"
            );
        }
    }

    #[test]
    fn renders_text_from_the_real_text_path() {
        let node = crate::Node {
            id: "t".into(),
            style: crate::Style {
                color: Some(0x000000),
                ..Default::default()
            },
            text: Some("Deka".into()),
            on_click: None,
            children: vec![],
        };
        let scene = crate::scene::Renderer::new().render(&node, 120., 40., 2.);
        assert!(!scene.images.is_empty(), "text produced glyph images");
        let shot = snapshot(&scene, 2.).expect("text render");
        let dark = shot
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] < 100 && p[1] < 100 && p[2] < 100)
            .count();
        assert!(dark > 50, "glyphs reached the target ({dark} dark pixels)");
    }
}

#[test]
fn a_resize_or_scale_change_rebuilds_the_scene_for_the_new_window() {
    struct Text;
    impl Application for Text {
        fn initial_state(&self) -> Vec<f64> {
            vec![]
        }
        fn render(&self, _: &[f64]) -> crate::Node {
            crate::Node {
                id: "p".into(),
                style: Default::default(),
                text: Some("Deka draws its own text and wraps it to the window".into()),
                on_click: None,
                children: vec![],
            }
        }
        fn event(&self, _: usize, _: &mut [f64]) {}
    }
    let mut content = ui::UiContent::new(Text, true);
    let lines = |scene: &Scene| {
        let mut tops: Vec<i32> = scene
            .paint
            .iter()
            .filter(|p| p.image.is_some())
            .map(|p| (p.rect.y + p.rect.height).round() as i32)
            .collect();
        tops.sort();
        tops.dedup_by(|a, b| (*a - *b).abs() < 4);
        tops.len()
    };
    let wide = content.frame(800., 200., 1.).clone();
    assert_eq!((wide.width, wide.height), (800., 200.));
    let narrow = content.frame(120., 200., 1.).clone();
    assert_eq!(narrow.width, 120.);
    assert!(
        lines(&narrow) > lines(&wide),
        "a narrower window wraps the text onto more lines"
    );
    let size = |scene: &Scene| {
        scene
            .images
            .iter()
            .map(|g| g.width * g.height)
            .sum::<usize>()
    };
    let dense = content.frame(800., 200., 2.).clone();
    assert!(
        size(&dense) > 3 * size(&wide),
        "at 2x the glyphs are rasterised with four times the pixels"
    );
}
