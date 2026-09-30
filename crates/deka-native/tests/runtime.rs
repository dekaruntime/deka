#![cfg(feature = "runtime")]
use deka_native::runtime::Session;
use deka_native_ir::Node;
use deka_native_ui::scene::Renderer;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn text(node: &Node) -> String {
    let mut output = node.text.clone().unwrap_or_default();
    for child in &node.children {
        output.push_str(&text(child));
    }
    output
}

fn compiler() -> PathBuf {
    std::env::var_os("DEKA_DSC")
        .map(PathBuf::from)
        .expect("runtime integration tests require DEKA_DSC pointing to the real dsc CLI")
}
fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
}

#[test]
fn compiled_components_state_styling_layout_and_unmount() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native/runtime");
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path();
    for name in ["app.dsx", "counter.dsx", "deka.json", "deka.lock"] {
        std::fs::copy(fixture.join(name), project.join(name)).unwrap();
    }
    let mut app = Session::open(project, &project.join("app.dsx"), &compiler(), "App").unwrap();
    let renderer = Renderer::new();
    let scene = renderer.render(app.tree(), 560., 300., 1.);
    assert_eq!(scene.targets.len(), 4);
    let first = &scene.targets[0];
    let second = &scene.targets[1];
    near(first.rect.x, 16.);
    near(first.rect.y, 16.);
    near(first.rect.width, 96.);
    near(first.rect.height, 64.);
    near(second.rect.x, 124.);
    near(second.rect.y, 16.);
    near(scene.targets[2].rect.y, 96.);
    near(scene.targets[3].rect.y, 160.);
    assert_eq!(scene.hit(20., 20.).unwrap().handler, first.handler);
    assert!(
        scene.hit(115., 20.).is_none(),
        "row gap must not be clickable"
    );
    app.click(first.handler).unwrap();
    let counts = text(app.tree());
    assert!(counts.contains("Count: 1"), "{counts}");
    assert_eq!(
        counts.matches("Count: 0").count(),
        2,
        "independent component state: {counts}"
    );
    let changed = renderer.render(app.tree(), 560., 300., 2.);
    let button = changed
        .paint
        .iter()
        .find(|p| p.image.is_none() && p.color == 0x663399)
        .unwrap();
    near(button.radius, 8.);
    near(button.rect.x, 16.);
    near(button.rect.width, 96.);
    assert!(
        changed.images.iter().any(|image| image
            .rgba
            .chunks_exact(4)
            .any(|rgba| rgba == [255, 255, 255, 255])),
        "inherited text color must reach glyph pixels"
    );
    let old_handler = scene.targets[3].handler;
    app.click(old_handler).unwrap();
    app.click(scene.targets[2].handler).unwrap();
    assert_eq!(renderer.render(app.tree(), 560., 300., 1.).targets.len(), 3);
    assert!(
        app.click(old_handler)
            .unwrap_err()
            .contains("no longer mounted")
    );
    app.click(scene.targets[2].handler).unwrap();
    let remounted = renderer.render(app.tree(), 560., 300., 1.);
    assert_ne!(remounted.targets[3].handler, old_handler);
    assert_eq!(
        text(app.tree()).matches("Count: 0").count(),
        2,
        "remounted child state must reset"
    );
    app.unmount().unwrap();
    assert!(
        renderer
            .render(app.tree(), 560., 300., 1.)
            .targets
            .is_empty()
    );
}

#[test]
fn runtime_timers_effect_cleanup_and_errors() {
    // Exercise actual React lifecycle and Deka host timer machinery. The DSX graph
    // above independently verifies DSC compilation and the real module loader.
    let project = tempfile::tempdir().unwrap();
    let entry = project.path().join("app.mjs");
    std::fs::write(
        &entry,
        r#"
      import React from '@js/react';
      export function App() {
        const [value, setValue] = React.useState('waiting');
        React.useEffect(() => {
          const timer = setTimeout(() => setValue('ready'), 15);
          return () => clearTimeout(timer);
        }, []);
        return React.createElement('button', {className: 'p-2', onClick: async () => {
          await new Promise(resolve => setTimeout(resolve, 10));
          setValue('clicked');
        }}, value);
      }
    "#,
    )
    .unwrap();
    let mut app = Session::open(project.path(), &entry, &compiler(), "App").unwrap();
    assert_eq!(text(app.tree()), "waiting");
    let deadline = Instant::now() + Duration::from_secs(3);
    while text(app.tree()) != "ready" && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        app.pump().unwrap();
    }
    assert_eq!(text(app.tree()), "ready");
    let handler = Renderer::new().render(app.tree(), 560., 300., 1.).targets[0].handler;
    app.click(handler).unwrap();
    assert_eq!(text(app.tree()), "ready", "handler must actually suspend");
    let deadline = Instant::now() + Duration::from_secs(3);
    while text(app.tree()) != "clicked" && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        app.pump().unwrap();
    }
    assert_eq!(text(app.tree()), "clicked");
    app.unmount().unwrap();
    assert_eq!(text(app.tree()), "");
}

#[test]
fn unsupported_props_styles_cleanup_and_permission_failures_are_reported() {
    let project = tempfile::tempdir().unwrap();
    let entry = project.path().join("app.mjs");
    for (body, expected) in [
        (
            "return React.createElement('input');",
            "Unsupported native element: input",
        ),
        (
            "return React.createElement('div', {style: {padding: 4}});",
            "Unsupported native prop: style",
        ),
        (
            "return React.createElement('div', {className: 'items-magic'});",
            "unsupported native utility: items-magic",
        ),
    ] {
        std::fs::write(
            &entry,
            format!("import React from '@js/react'; export function App() {{ {body} }}"),
        )
        .unwrap();
        let error = Session::open(project.path(), &entry, &compiler(), "App")
            .err()
            .expect("unsupported UI must fail explicitly");
        assert!(error.contains(expected), "{error}");
    }
    std::fs::write(project.path().join("deka.json"), r#"{"security":{"allow":{"read":true}},"permissions":{"dev":{"read":true},"prod":{"read":false}}}"#).unwrap();
    std::fs::write(&entry, r#"
        import React from '@js/react';
        export function App() {
            if (globalThis.Deno !== undefined) throw new Error('raw host API leaked');
            React.useEffect(() => () => { throw new Error('cleanup-ran'); }, []);
            const denied = globalThis[Symbol.for('deka.host.internal')].host('fs', 'read_file_sync', ['/native-denied-probe'], ['fs']);
            return React.createElement('span', {}, JSON.stringify(denied));
        }
    "#).unwrap();
    let mut app = Session::open(project.path(), &entry, &compiler(), "App").unwrap();
    let denied = text(app.tree());
    assert!(
        denied.contains("PermissionDenied"),
        "real host policy must deny access: {denied}"
    );
    let error = app.unmount().unwrap_err();
    assert!(
        error.contains("cleanup-ran"),
        "effect cleanup must execute: {error}"
    );
}

#[test]
fn component_state_updates_layout_and_text_measurement() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native/runtime");
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path();
    for name in ["layout.dsx", "deka.json", "deka.lock"] {
        std::fs::copy(fixture.join(name), project.join(name)).unwrap();
    }
    let mut app = Session::open(project, &project.join("layout.dsx"), &compiler(), "App").unwrap();
    let renderer = Renderer::new();
    let first = renderer.render(app.tree(), 560., 300., 1.);
    let button = &first.targets[0];
    near(button.rect.x, 16.);
    near(button.rect.y, 16.);
    app.click(button.handler).unwrap();
    let centered = renderer.render(app.tree(), 560., 300., 1.);
    near(centered.targets[0].rect.x, 184.);
    assert!(centered.targets[0].rect.y > 16.);
    assert!(
        centered.hit(20., 20.).is_none(),
        "old location must not remain clickable"
    );
    let text = centered
        .nodes
        .iter()
        .find(|n| n.text.as_ref().is_some_and(|t| t.starts_with("This text")))
        .unwrap();
    assert!(
        text.rect.height > 32.,
        "wrapped text reserves multiple lines"
    );
    app.click(button.handler).unwrap();
    let restored = renderer.render(app.tree(), 560., 300., 1.);
    near(restored.targets[0].rect.x, 16.);
    near(restored.targets[0].rect.y, 16.);
}

#[test]
fn react_commits_targets_once_and_rust_animates_between_commits() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native/runtime");
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path();
    for name in ["animation.dsx", "deka.json", "deka.lock"] {
        std::fs::copy(fixture.join(name), project.join(name)).unwrap();
    }
    let mut app =
        Session::open(project, &project.join("animation.dsx"), &compiler(), "App").unwrap();
    let renderer = Renderer::new();
    let first = renderer.render_at(app.tree(), 560., 300., 1., 0., false);
    let id = first.targets[0].id.clone();
    let handler = first.targets[0].handler;
    near(first.targets[0].rect.width, 160.);
    app.click(handler).unwrap();
    assert_eq!(
        renderer.render(app.tree(), 560., 300., 1.).targets[0]
            .rect
            .width,
        256.
    );
    renderer.render_at(app.tree(), 560., 300., 1., 100., false);
    // No JS event or pump occurs between these samples.
    let half = renderer.render_at(app.tree(), 560., 300., 1., 400., false);
    assert_eq!(half.targets[0].id, id);
    near(half.targets[0].rect.width, 208.);
    assert!(half.animating);
    app.click(handler).unwrap();
    near(
        renderer
            .render_at(app.tree(), 560., 300., 1., 400., false)
            .targets[0]
            .rect
            .width,
        208.,
    );
    let end = renderer.render_at(app.tree(), 560., 300., 1., 1000., false);
    near(end.targets[0].rect.width, 160.);
    assert!(!end.animating);
}

#[test]
fn react_removes_hosts_while_rust_finishes_inert_exit() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native/runtime");
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path();
    for name in ["motion.dsx", "deka.json", "deka.lock"] {
        std::fs::copy(fixture.join(name), project.join(name)).unwrap();
    }
    let mut app = Session::open(project, &project.join("motion.dsx"), &compiler(), "App").unwrap();
    let r = Renderer::new();
    let first = r.render_at(app.tree(), 500., 400., 1., 0., false);
    let handler = first.targets[0].handler;
    app.click(handler).unwrap();
    assert!(text(app.tree()).contains("Hello again"));
    assert!(
        r.render_at(app.tree(), 500., 400., 1., 10., false)
            .animating
    );
    r.render_at(app.tree(), 500., 400., 1., 1000., false);
    app.click(handler).unwrap();
    assert!(!text(app.tree()).contains("Hello again"));
    let exit = r.render_at(app.tree(), 500., 400., 1., 1100., false);
    assert!(
        exit.nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Hello again"))
    );
    assert!(exit.animating);
    let end = r.render_at(app.tree(), 500., 400., 1., 1800., false);
    assert!(
        !end.nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Hello again"))
    );
    assert!(!end.animating);
}
