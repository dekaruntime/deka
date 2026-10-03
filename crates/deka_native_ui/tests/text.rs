//! Text measuring and wrapping as the layout pass sees it, through the public scene API.
use deka_native_ui::{Edges, Length, Node, Style, scene::Renderer};

const PARAGRAPH: &str = "Deka draws its own text: shaping, line breaking and fallback all happen before the GPU sees a single glyph.";

fn text(content: &str, size: f32, nowrap: bool) -> Node {
    Node {
        id: content.into(),
        style: Style {
            font_size: Some(size),
            nowrap: Some(nowrap),
            ..Default::default()
        },
        text: Some(content.into()),
        on_click: None,
        children: vec![],
    }
}
fn column(width: f32, children: Vec<Node>) -> Node {
    Node {
        id: "column".into(),
        style: Style {
            width: Length::Px(width),
            padding: Edges::all(10.),
            align: deka_native_ui::Align::Start,
            ..Default::default()
        },
        text: None,
        on_click: None,
        children,
    }
}

#[test]
fn text_boxes_take_their_measured_size_and_wrap_at_the_container() {
    let root = column(
        320.,
        vec![
            text("Hamburgefonstiv", 16., true),
            text(PARAGRAPH, 14., false),
        ],
    );
    for scale in [1., 2.] {
        let scene = Renderer::new().render(&root, 640., 480., scale);
        let rect = |id: &str| scene.nodes.iter().find(|n| n.id == id).unwrap().layout_rect;
        let label = rect("Hamburgefonstiv");
        assert_eq!((label.width, label.height), (120., 20.), "@{scale}x");
        let paragraph = rect(PARAGRAPH);
        // 300 px of content width: three lines of 14 px Atkinson Hyperlegible (17.36 px each),
        // the widest 287 px wide.
        assert_eq!(paragraph.width, 287., "@{scale}x");
        assert_eq!(paragraph.height, 52., "@{scale}x");
        assert_eq!(paragraph.y, label.y + label.height);
    }
}

#[test]
fn min_content_never_breaks_inside_a_word() {
    // A zero-width column shrinks text to min-content: its longest word, on its own line.
    let mut root = column(0., vec![text("a breaking point", 14., false)]);
    root.style.padding = Edges::default();
    let scene = Renderer::new().render(&root, 640., 480., 1.);
    let item = scene
        .nodes
        .iter()
        .find(|n| n.id == "a breaking point")
        .unwrap();
    let single = Renderer::new().render(
        &column(640., vec![text("breaking", 14., true)]),
        640.,
        480.,
        1.,
    );
    let word = single.nodes[1].layout_rect.width;
    assert!(word > 40.);
    assert_eq!(item.layout_rect.width, word);
    assert_eq!(item.layout_rect.height.round(), (17.36_f32 * 3.).round());
}

#[test]
fn every_visible_glyph_is_painted_once() {
    let scene = Renderer::new().render(&text("Hi there", 16., true), 200., 40., 2.);
    let glyphs = scene.paint.iter().filter(|p| p.image.is_some()).count();
    assert_eq!(glyphs, 7);
    for paint in scene.paint.iter().filter(|p| p.image.is_some()) {
        let id = paint.image.as_ref().unwrap();
        let image = scene.images.iter().find(|i| &i.id == id).unwrap();
        assert_eq!(image.width as f32, paint.rect.width * 2.);
        assert!(
            image.rgba.chunks_exact(4).any(|p| p[3] > 200),
            "{id} has ink"
        );
    }
}
