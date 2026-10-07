use deka_ui::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn clicks_patch_only_the_bound_property_of_the_retained_node() {
    let evaluations = Rc::new(Cell::new(0));
    let observed = evaluations.clone();
    let app = UiApp::new(move || {
        let mut count = signal(0);
        let mut noise = signal(0);
        View::element("view")
            .child(
                View::element("p")
                    .attr("className", "text-xl")
                    .child(View::live_text(move || {
                        observed.set(observed.get() + 1);
                        count.get()
                    })),
            )
            .child(View::element("p").child("Unchanged"))
            .child(
                View::element("button")
                    .on_click(move |_| noise += 1)
                    .child("Noise"),
            )
            .child(
                View::element("button")
                    .on_click(move |_| count += 1)
                    .child("Count"),
            )
    });
    let before = app.tree();
    assert_eq!(evaluations.get(), 1);
    assert!(app.dispatch(0));
    assert_eq!(app.tree(), before);
    assert_eq!(evaluations.get(), 1);
    assert_eq!(app.patch_passes(), 0);
    assert!(app.dispatch(1));
    let after = app.tree();
    assert_eq!(after.id, before.id);
    assert_eq!(after.children[0].id, before.children[0].id);
    assert_eq!(after.children[0].style, before.children[0].style);
    assert_eq!(
        after.children[0].children[0].id,
        before.children[0].children[0].id
    );
    assert_eq!(after.children[0].children[0].text.as_deref(), Some("1"));
    assert_eq!(after.children[1..], before.children[1..]);
    assert_eq!(evaluations.get(), 2);
    assert_eq!(app.property_patches(), 1);
    assert_eq!(app.patch_passes(), 1);
    for _ in 0..60 {
        assert_eq!(app.tree(), after);
    }
    assert_eq!(evaluations.get(), 2);
}

#[test]
fn an_event_batches_multiple_writes_into_one_final_binding_pass() {
    let values = Rc::new(RefCell::new(vec![]));
    let seen = values.clone();
    let app = UiApp::new(move || {
        let count = signal(0);
        View::element("view")
            .child(View::live_text(move || {
                let value = count.get();
                seen.borrow_mut().push(value);
                value
            }))
            .child(
                View::element("button")
                    .on_click(move |_| {
                        count.set(1);
                        count.set(2);
                    })
                    .child("Two writes"),
            )
    });
    app.tree();
    app.dispatch(0);
    assert_eq!(*values.borrow(), [0, 2]);
    assert_eq!(app.tree().children[0].text.as_deref(), Some("2"));
    assert_eq!(app.property_patches(), 1);
    assert_eq!(app.patch_passes(), 1);
}

#[test]
fn option_and_iterator_children_preserve_later_static_siblings() {
    let app = UiApp::new(|| {
        let visible = signal(true);
        View::element("view")
            .child(move || visible.get().then(|| View::element("p").child("Optional")))
            .children(
                ["First", "Second"]
                    .into_iter()
                    .map(|text| View::element("p").child(text)),
            )
            .child(View::element("p").child("Last"))
            .child(
                View::element("button")
                    .on_click(move |_| visible.toggle())
                    .child("Toggle"),
            )
    });
    let before = app.tree();
    app.dispatch(0);
    let hidden = app.tree();
    assert_eq!(hidden.children, before.children[1..]);
    app.dispatch(0);
    let shown = app.tree();
    assert_eq!(shown.children[1..], before.children[1..]);
    assert_ne!(shown.children[0].id, before.children[0].id);
    assert_eq!(
        shown.children[0].children[0].text.as_deref(),
        Some("Optional")
    );
}

#[test]
fn class_toggles_and_dynamic_attributes_patch_without_rebuilding_other_properties() {
    let app = UiApp::new(|| {
        let open = signal(false);
        let name = signal("before");
        View::element("view")
            .child(
                View::element("p")
                    .attr("id", move || name.get())
                    .attr("className", "p-4")
                    .class("opacity-0", open)
                    .child("Kept"),
            )
            .child(
                View::element("button")
                    .on_click(move |_| {
                        open.toggle();
                        name.set("after");
                    })
                    .child("Change"),
            )
    });
    let before = app.tree();
    app.dispatch(0);
    let after = app.tree();
    assert_eq!(after.children[0].id, before.children[0].id);
    assert_eq!(
        after.children[0].style.padding,
        before.children[0].style.padding
    );
    assert_eq!(after.children[0].style.opacity, 0.);
    assert_eq!(after.children[0].children, before.children[0].children);
    assert_eq!(after.children[1], before.children[1]);
    assert_eq!(app.property_patches(), 2);
    assert_eq!(app.patch_passes(), 1);
}

#[test]
fn removed_bindings_stop_observing_and_event_captures_are_released() {
    let evaluations = Rc::new(Cell::new(0));
    let seen = evaluations.clone();
    let captured = Rc::new(());
    let weak = Rc::downgrade(&captured);
    let app = UiApp::new(move || {
        let visible = signal(true);
        let mut count = signal(0);
        // Ownership enters the structural getter once; later removals drop its
        // mounted listener and binding while the permanent controls stay live.
        let capture = signal(Some(captured));
        View::element("view")
            .child(move || {
                let seen = seen.clone();
                visible.get().then(|| {
                    let held = capture.get().unwrap();
                    View::element("button")
                        .on_click(move |_| {
                            let _keep = &held;
                        })
                        .child(View::live_text(move || {
                            seen.set(seen.get() + 1);
                            count.get()
                        }))
                })
            })
            .child(
                View::element("button")
                    .on_click(move |_| {
                        visible.set(false);
                        capture.set(None);
                    })
                    .child("Remove"),
            )
            .child(
                View::element("button")
                    .on_click(move |_| count += 1)
                    .child("Count"),
            )
    });
    assert!(weak.upgrade().is_some());
    app.tree();
    app.dispatch(1);
    let hidden = app.tree();
    assert_eq!(hidden.children.len(), 2);
    assert!(weak.upgrade().is_none());
    let before = evaluations.get();
    app.dispatch(1);
    assert_eq!(evaluations.get(), before);
    assert_eq!(app.tree(), hidden);
}

#[test]
fn typed_input_and_key_events_use_the_same_batched_binding_path() {
    let app = UiApp::new(|| {
        let text = signal(String::new());
        View::element("view")
            .child(
                View::element("input")
                    .attr("value", move || text.get())
                    .on(EventKind::Input, move |event| {
                        if let Event::Input(value) = event {
                            text.set(value);
                        }
                    })
                    .on(EventKind::KeyDown, move |event| {
                        if event == Event::KeyDown("Escape".into()) {
                            text.set(String::new());
                        }
                    }),
            )
            .child(text)
    });
    let before = app.tree();
    let id = &before.children[0].id;
    assert!(app.dispatch_to(id, Event::Input("Hello".into())));
    assert_eq!(app.tree().children[1].text.as_deref(), Some("Hello"));
    assert!(app.dispatch_to(id, Event::KeyDown("Escape".into())));
    assert_eq!(app.tree().children[1].text.as_deref(), Some(""));
    assert!(!app.dispatch_to(id, Event::Click));
    assert_eq!(app.patch_passes(), 2);
}

#[test]
fn the_existing_application_host_executes_real_rust_event_closures() {
    let app = UiApp::new(|| {
        let mut count = signal(0);
        View::element("view").child(count).child(
            View::element("button")
                .on_click(move |_| count += 1)
                .child("Add"),
        )
    });
    assert_eq!(deka_native_ui::exercise(app, 3), "3 Add");
}

#[test]
fn dropped_app_releases_scope_values_and_stale_signals_cannot_alias() {
    let held = Rc::new(RefCell::new(None));
    let result = held.clone();
    let value = Rc::new(());
    let weak = Rc::downgrade(&value);
    let app = UiApp::new(move || {
        let state = signal(value);
        *result.borrow_mut() = Some(state);
        View::element("view").child("Owned")
    });
    drop(app);
    assert!(weak.upgrade().is_none());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| held
            .borrow()
            .unwrap()
            .get()))
        .is_err()
    );
}
