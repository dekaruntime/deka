use super::*;
fn step(w: &mut World, start: f64, count: usize) {
    for i in 0..=count {
        w.advance(start + i as f64 * 1000. / 120., false);
    }
}
#[test]
fn movement_collision_and_focus_loss() {
    let mut w = World::new();
    w.start();
    w.key("ArrowUp", true);
    step(&mut w, 0., 240);
    assert!(
        (w.y - 295.).abs() < 1.,
        "wall should stop feet below house: {}",
        w.y
    );
    assert!(w.snapshot().hint.contains("Deka workshop"));
    w.blur();
    let y = w.y;
    step(&mut w, 3000., 120);
    assert_eq!(w.y, y);
    assert!(!w.walking);
}
#[test]
fn diagonal_speed_and_frame_cadence_are_consistent() {
    let mut a = World::new();
    a.start();
    a.key("ArrowRight", true);
    step(&mut a, 0., 120);
    let mut b = World::new();
    b.start();
    b.key("ArrowRight", true);
    for i in 0..=30 {
        b.advance(i as f64 * 1000. / 30., false);
    }
    assert!((a.x - b.x).abs() < 0.001);
    assert!((a.x - 344.).abs() < 0.01);
    let mut c = World::new();
    c.start();
    c.key("ArrowDown", true);
    c.key("ArrowRight", true);
    step(&mut c, 0., 60);
    assert!(((c.x - 244.).hypot(c.y - 350.) - 50.).abs() < 0.02);
    let before = b.x;
    b.advance(60000., false);
    assert!(b.x - before <= 10.01);
}
#[test]
fn door_fades_change_room_once_and_return_to_same_house() {
    let mut w = World::new();
    w.start();
    w.key("ArrowUp", true);
    step(&mut w, 0., 80);
    w.key("ArrowUp", false);
    w.key("Enter", true);
    assert!(w.transition.is_some());
    step(&mut w, 670., 70);
    assert_eq!(w.room, Some(0));
    assert!(w.transition.is_none());
    w.key("Enter", true);
    assert!(w.transition.is_none(), "key repeat must not leave room");
    assert!(w.frame(960., 640., 1300., false).paint.len() > 50);
    w.key("Enter", false);
    w.key("Enter", true);
    step(&mut w, 1310., 70);
    assert_eq!(w.room, None);
    assert!((w.y - 318.).abs() < 0.1);
    assert!(w.drain_sounds().contains(&1));
    assert!(w.drain_sounds().is_empty());
}
#[test]
fn depth_order_changes_when_walking_behind_tree() {
    let mut w = World::new();
    w.x = 122.;
    w.y = 249.;
    let behind = w.frame(480., 320., 0., false);
    let hero = behind
        .paint
        .iter()
        .position(|p| p.image.as_deref() == Some("world-hero-0-0"))
        .unwrap();
    assert!(
        behind.paint[hero + 1..]
            .iter()
            .any(|p| p.image.as_deref() == Some("world-tree"))
    );
    // Same tree, player in front; identify the tree's screen rectangle, not all trees.
    w.y = 285.;
    let front = w.frame(480., 320., 0., false);
    let tree = front
        .paint
        .iter()
        .position(|p| p.image.as_deref() == Some("world-tree") && p.rect.x == 92.)
        .unwrap();
    let hero = front
        .paint
        .iter()
        .position(|p| p.image.as_deref() == Some("world-hero-0-0"))
        .unwrap();
    assert!(tree < hero);
}
#[test]
fn pcm_is_finite_non_silent_and_mute_clears_pending_effects() {
    for id in 0..3 {
        let samples = audio::samples(id);
        assert!(samples.iter().all(|s| s.is_finite() && s.abs() < 1.));
        assert!(samples.iter().any(|s| s.abs() > 0.01));
    }
    let mut w = World::new();
    w.start();
    w.key("s", true);
    step(&mut w, 0., 60);
    assert!(!w.sounds.is_empty());
    w.set_muted(true);
    assert!(w.drain_sounds().is_empty());
    step(&mut w, 510., 60);
    assert!(w.drain_sounds().is_empty());
}
#[test]
fn reduced_motion_skips_fade_and_invalid_clock_does_not_poison_world() {
    let mut w = World::new();
    w.start();
    w.y = 300.;
    w.interact();
    w.advance(0., true);
    w.advance(10., true);
    assert_eq!(w.room, Some(0));
    assert!(!w.snapshot().transitioning);
    w.advance(f64::NAN, false);
    w.advance(-1000., false);
    assert!(
        w.frame(480., 320., 20., true)
            .paint
            .iter()
            .all(|p| p.opacity.is_finite())
    );
}
