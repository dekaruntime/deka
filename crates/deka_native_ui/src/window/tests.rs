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

    /// Frame one is drawn straight into the window layer's own drawable,
    /// before the window is shown (wgpu would not hand out a surface texture
    /// yet). That drawable, wrapped for wgpu, must take the frame in the
    /// layer's size and channel order.
    #[test]
    fn frame_one_draws_into_the_layers_own_drawable() {
        let layer = objc2_quartz_core::CAMetalLayer::new();
        let mut gpu = Gpu::new().expect("a GPU");
        // SAFETY: `layer` is a live CAMetalLayer that outlives `surface`
        // (dropped first, below).
        let surface = unsafe {
            gpu.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
                    objc2::rc::Retained::as_ptr(&layer).cast_mut().cast(),
                ))
        }
        .expect("a surface on the layer");
        let format = gpu.adopt(&surface).expect("a format the renderer draws");
        let config = wgpu::SurfaceConfiguration {
            // COPY_SRC only so the test can read the drawable back.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width: 200,
            height: 160,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&gpu.device, &config);
        let (drawable, texture) = gpu
            .layer_texture(&surface, &config)
            .expect("the layer's next drawable");
        let view = texture.create_view(&Default::default());
        gpu.draw(&sample(), 2., &view, 200, 160).expect("frame one");
        let shot = gpu.read_back(&texture).expect("the drawable read back");
        assert_eq!((shot.width, shot.height), (200, 160));
        assert_pixel(&shot, 2, 2, [255, 255, 255, 255]);
        assert_pixel(&shot, 40, 40, [255, 0, 0, 255]);
        assert_pixel(&shot, 100, 40, [127, 127, 255, 255]);
        assert_pixel(&shot, 171, 22, [0, 200, 0, 255]);
        gpu.present_drawable(&drawable).expect("presented");
        drop(surface);
    }

    #[test]
    fn upscaled_images_are_filtered_up_to_their_edges() {
        // Black, white, white, black, drawn 4x wider (8 logical px at 2x). Bilinear
        // filtering with edge padding is symmetric and ends on the edge texels.
        let image = GlyphImage {
            id: "i".into(),
            width: 4,
            height: 1,
            rgba: [0u8, 255, 255, 0]
                .iter()
                .flat_map(|&v| [v, v, v, 255])
                .collect(),
        };
        let scene = Scene {
            width: 16.,
            height: 4.,
            background: 0xff0000,
            paint: vec![Paint {
                rect: rect(0., 0., 8., 2.),
                clip: rect(0., 0., 16., 4.),
                color: 0,
                radius: 0.,
                image: Some("i".into()),
                opacity: 1.,
            }],
            images: vec![image],
            ..Default::default()
        };
        let shot = snapshot(&scene, 2.).expect("render");
        let row: Vec<u8> = (0..16).map(|x| shot.pixel(x, 1)[1]).collect();
        let expected = [
            0, 0, 32, 96, 159, 223, 255, 255, 255, 255, 223, 159, 96, 32, 0, 0,
        ];
        assert!(
            row.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2),
            "row {row:?}, expected {expected:?}"
        );
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
