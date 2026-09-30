use deka_native_ui::{Node, Style, animation::Animator, scene::Renderer};
fn node(id: &str, classes: &str) -> Node {
    let mut style = Style::default();
    deka_native_ir::apply_classes(&mut style, classes).unwrap();
    Node {
        id: id.into(),
        style,
        text: None,
        on_click: Some(0),
        children: vec![],
    }
}
#[test]
fn spring_overshoots_stays_active_and_settles() {
    let mut a = Animator::default();
    let mut n = node("n", "transition-transform spring spring-damping-8");
    a.sample(&n, 0., false);
    n.style.translate_x = 100.;
    a.sample(&n, 0., false);
    let (mid, active) = a.sample(&n, 250., false);
    assert!(mid.style.translate_x > 100. && active);
    let (end, active) = a.sample(&n, 10000., false);
    assert_eq!(end.style.translate_x, 100.);
    assert!(!active);
}
#[test]
fn keyframes_repeat_alternate_delay_and_reduced_motion() {
    let mut a = Animator::default();
    let n = node(
        "n",
        "frames-x-[0:0,100:100] duration-1000 delay-100 repeat-2 alternate",
    );
    assert_eq!(a.sample(&n, 0., false).0.style.translate_x, 0.);
    assert_eq!(a.sample(&n, 600., false).0.style.translate_x, 50.);
    assert_eq!(a.sample(&n, 1600., false).0.style.translate_x, 50.);
    let (end, active) = a.sample(&n, 2100., false);
    assert_eq!(end.style.translate_x, 0.);
    assert!(!active);
    let forever = node("n", "animate-spin repeat-infinite duration-1000");
    assert!(a.sample(&forever, 2200., false).1);
    let (reduced, active) = a.sample(&forever, 2300., true);
    assert_eq!(reduced.style.rotate, 0.);
    assert!(!active);
}
#[test]
fn presence_retains_inert_visuals_then_releases_and_staggers_mounts() {
    let mut a = Animator::default();
    let mut root = node("root", "stagger-100");
    root.children = vec![
        node("a", "enter-fade exit-fade duration-1000 ease-linear"),
        node("b", "enter-fade exit-fade duration-1000 ease-linear"),
    ];
    a.sample(&root, 0., false);
    let (s, active) = a.sample(&root, 100., false);
    assert!(active);
    assert_eq!(s.children[0].style.opacity, 0.1);
    assert_eq!(s.children[1].style.opacity, 0.);
    a.sample(&root, 1100., false);
    root.children.clear();
    let (exit, active) = a.sample(&root, 1200., false);
    assert!(active);
    assert_eq!(exit.children.len(), 2);
    assert!(exit.children.iter().all(|n| n.on_click.is_none()));
    assert_eq!(
        a.sample(&root, 1700., false).0.children[0].style.opacity,
        0.5
    );
    assert!(a.sample(&root, 2200., false).0.children.is_empty());
}
#[test]
fn layout_reorders_retargets_and_snaps_for_reduced_motion() {
    let r = Renderer::new();
    let mut root = node("root", "w-80 h-40 flex-row");
    root.children = vec![
        node(
            "a",
            "w-20 h-20 shrink-0 transition-layout duration-1000 ease-linear",
        ),
        node(
            "b",
            "w-20 h-20 shrink-0 transition-layout duration-1000 ease-linear",
        ),
    ];
    let x = |s: deka_native_ui::scene::Scene| s.nodes.iter().find(|n| n.id == "a").unwrap().rect.x;
    assert_eq!(x(r.render_at(&root, 400., 240., 1., 0., false)), 0.);
    root.children.swap(0, 1);
    assert_eq!(x(r.render_at(&root, 400., 240., 1., 100., false)), 0.);
    assert_eq!(x(r.render_at(&root, 400., 240., 1., 600., false)), 40.);
    root.children.swap(0, 1);
    assert_eq!(x(r.render_at(&root, 400., 240., 1., 600., false)), 40.);
    assert_eq!(x(r.render_at(&root, 400., 240., 1., 1100., false)), 20.);
    let s = r.render_at(&root, 400., 240., 1., 1200., true);
    assert!(!s.animating);
    assert_eq!(x(s), 0.);
}
#[test]
fn rotated_pixels_and_hits_follow_the_same_shape() {
    let r = Renderer::new();
    let mut root = node("root", "w-80 h-80 p-20");
    root.on_click = None;
    root.children
        .push(node("rotated", "w-20 h-20 shrink-0 rotate-45 bg-[#663399]"));
    let s = r.render(&root, 320., 320., 1.);
    assert!(s.hit(120., 120.).is_some());
    assert!(
        s.hit(65., 65.).is_none(),
        "AABB corner is outside the diamond"
    );
    let image = s
        .images
        .iter()
        .find(|i| i.id.starts_with("affine-"))
        .unwrap();
    assert!(image.rgba.chunks_exact(4).any(|p| p[3] == 0));
    assert!(image.rgba.chunks_exact(4).any(|p| p[3] == 255));
    root.style.clip = true;
    root.style.padding = deka_native_ui::Edges::all(0.);
    root.children[0].style.margin = deka_native_ui::Edges::all(80.);
    root.style.width = deka_native_ui::Length::Px(100.);
    assert!(r.render(&root, 320., 320., 1.).hit(120., 120.).is_none());
}
