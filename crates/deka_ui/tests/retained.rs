use deka_native_ui::{Application, Waker, scene::Renderer};
use deka_ui::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
fn node_refs_reads_snapshots_and_detached_lifetimes_port_lane_b() {
    let reference = node_ref();
    assert!(reference.get().is_none());
    let mounted = reference.clone();
    let signals = Rc::new(Cell::new(None));
    let output = signals.clone();
    let app = UiApp::new(move || {
        let visible = signal(true);
        let title = signal("First");
        output.set(Some((visible, title)));
        view! { <view id="root"><div id="section" className="p-4">
            {move || visible.get().unwrap().then(|| view! {
                <p id="message" node_ref={mounted.clone()} className="text-sm">
                    {title}<span>" child"</span>
                </p>
            })}
        </div></view> }
    });
    let (visible, title) = signals.get().unwrap();
    let node = reference.get().unwrap();
    let element = node.as_element().unwrap();
    assert_eq!(node.text_content(), "First child");
    assert_eq!(node.parent_node().unwrap().text_content(), "First child");
    assert_eq!(
        element.get_attribute("className").as_deref(),
        Some("text-sm")
    );
    assert_eq!(element.get_attribute("not-an-attribute"), None);
    assert_eq!(element.class_list(), element.class_list());
    let mut children = element.children();
    let snapshot = children.clone();
    children.push(element.clone());
    assert_eq!(element.children(), snapshot);
    assert_eq!(node.child_nodes().len(), 2);
    assert!(node.child_nodes()[0].as_element().is_none());
    title.set("Changed");
    assert_eq!(node.text_content(), "Changed child");
    assert_eq!(app.tree().get_element_by_id("message").unwrap(), element);
    visible.set(false);
    assert!(app.tree().get_element_by_id("message").is_none());
    assert!(node.parent_node().is_none());
    assert_eq!(node.text_content(), "Changed child");
    visible.set(true);
    assert_ne!(reference.get().unwrap(), node);
    assert_eq!(snapshot[0].text_content(), " child");
    drop(app);
    assert_eq!(node.text_content(), "Changed child");
    node.set_text_content("Detached").unwrap();
    assert_eq!(node.text_content(), "Detached");
}

#[test]
fn selectors_port_compounds_combinators_backtracking_order_and_live_identity() {
    let signals = Rc::new(Cell::new(None));
    let output = signals.clone();
    let app = UiApp::new(move || {
        let visible = signal(true);
        let id = signal("first");
        let classes = signal("p-4 text-sm");
        output.set(Some((visible, id, classes)));
        view! {<view id="root">
            <div className="p-4"><p id={move || id.get().unwrap()} className={move || classes.get().unwrap()}>"First"<span>" child"</span></p></div>
            <div className="p-4"><div><div className="p-4">
                {move || visible.get().unwrap().then(|| view!{<p id="second" className="text-sm">"Second"</p>})}
            </div></div></div>"Text"
        </view>}
    });
    let tree = app.tree();
    let first = tree.get_element_by_id("first").unwrap();
    let second = tree.get_element_by_id("second").unwrap();
    assert_eq!(
        tree.query_all("p").unwrap(),
        [first.clone(), second.clone()]
    );
    for selector in [
        "p#first.p-4.text-sm",
        "#root > .p-4 > p",
        "view > .p-4 .text-sm",
    ] {
        assert_eq!(
            tree.query(selector).unwrap(),
            Some(first.clone()),
            "{selector}"
        );
    }
    assert_eq!(
        tree.query_all("view > .p-4 .text-sm").unwrap(),
        [first.clone(), second.clone()]
    );
    assert_eq!(
        tree.query_all("view > .p-4 > .text-sm").unwrap(),
        std::slice::from_ref(&first)
    );
    assert_eq!(
        tree.query_all("#root").unwrap(),
        [tree.get_element_by_id("root").unwrap()]
    );
    assert!(tree.query_all(".p").unwrap().is_empty());
    assert!(tree.query("#absent").unwrap().is_none());
    let snapshot = tree.query_all("p").unwrap();
    let (visible, id, classes) = signals.get().unwrap();
    id.set("renamed");
    classes.set("p-8");
    assert!(tree.query("#first").unwrap().is_none());
    assert_eq!(tree.query_all("p#renamed.p-8").unwrap(), [first]);
    assert_eq!(
        tree.query_all(".text-sm").unwrap(),
        std::slice::from_ref(&second)
    );
    visible.set(false);
    assert_eq!(tree.query_all("p").unwrap().len(), 1);
    assert_eq!(snapshot[1], second);
    assert!(snapshot[1].parent_node().is_none());
}

#[test]
fn selectors_reject_unsupported_syntax_even_before_mounting() {
    assert!(matches!(tree(), Err(ViewError::NoActiveApp)));
    let app = UiApp::new(|| {
        let tree = tree().unwrap();
        assert!(tree.query("p").unwrap().is_none());
        for selector in [
            "",
            " ",
            "*",
            "p,div",
            "[id]",
            "p:hover",
            "p + div",
            "p ~ div",
            "p >",
            ".1x",
            "p/",
            "#",
            "p >> span",
            ".bg-[red]",
            "\\p",
        ] {
            assert!(
                tree.query(selector)
                    .unwrap_err()
                    .to_string()
                    .starts_with("invalid selector at byte ")
            );
            assert!(tree.query_all(selector).is_err());
        }
        view! {<view/>}
    });
    assert!(app.tree().query("*").is_err());
}

#[test]
fn text_attribute_and_class_edits_keep_authored_ownership_and_reclaim_on_change() {
    let signals = Rc::new(Cell::new(None));
    let output = signals.clone();
    let app = UiApp::new(move || {
        let count = signal(0);
        let open = signal(false);
        let id = signal("message");
        output.set(Some((count, open, id)));
        view! {<view><p id={move || id.get().unwrap()} value={move || count.get().unwrap()%2}
            className={move || if open.get().unwrap(){"p-4"}else{"p-2"}}>
            {move || count.get().unwrap()%2}<span>" child"</span>
        </p></view>}
    });
    let (count, open, id) = signals.get().unwrap();
    let element = app.tree().get_element_by_id("message").unwrap();
    let authored_children = element.child_nodes();
    element.set_text_content("Edited").unwrap();
    element.set_attribute("value", "Manual").unwrap();
    element.class_list().add("opacity-25").unwrap();
    assert_eq!(element.text_content(), "Edited");
    assert!(element.children().is_empty());
    assert!(
        authored_children
            .iter()
            .all(|node| node.parent_node().is_none())
    );
    assert_eq!(app.tree().children[0].style.opacity, 0.25);
    let edited = app.tree();
    count.set(2);
    assert_eq!(app.tree(), edited);
    id.set("renamed");
    assert_eq!(element.text_content(), "Edited");
    assert_eq!(element.get_attribute("value").as_deref(), Some("Manual"));
    assert!(element.class_list().contains("opacity-25"));
    count.set(3);
    assert_eq!(element.text_content(), "1 child");
    assert_eq!(element.get_attribute("value").as_deref(), Some("1"));
    assert_eq!(element.child_nodes(), authored_children);
    assert!(element.class_list().contains("opacity-25"));
    element.set_text_content("Again").unwrap();
    open.toggle();
    assert_eq!(element.text_content(), "Again");
    assert_eq!(element.class_list().tokens(), ["p-4"]);
    assert_eq!(app.tree().children[0].style.opacity, 1.);
    count.set(5);
    assert_eq!(element.text_content(), "Again");
    count.set(6);
    assert_eq!(element.text_content(), "0 child");
}

#[test]
fn edits_wake_once_for_the_next_frame_and_detached_or_unchanged_edits_do_not() {
    let reference = node_ref();
    let mounted = reference.clone();
    let visible = Rc::new(Cell::new(None));
    let output = visible.clone();
    let mut app = UiApp::new(move || {
        let shown = signal(true);
        output.set(Some(shown));
        view! {<view>{move || shown.get().unwrap().then(|| view!{<p node_ref={mounted.clone()}>"Original"</p>})}</view>}
    });
    let wakes = Arc::new(AtomicUsize::new(0));
    let output = wakes.clone();
    app.set_waker(Waker::new(move || {
        output.fetch_add(1, Ordering::SeqCst);
    }));
    let node = reference.get().unwrap().as_element().unwrap();
    let before = Renderer::new().render_at(&app.tree(), 320., 240., 1., 0., false);
    node.set_text_content("Edited").unwrap();
    node.set_attribute("className", "p-4").unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    let after = Renderer::new().render_at(&app.tree(), 320., 240., 1., 0., false);
    assert!(
        after
            .nodes
            .iter()
            .any(|node| node.text.as_deref() == Some("Edited"))
    );
    assert!(
        before
            .nodes
            .iter()
            .all(|node| node.text.as_deref() != Some("Edited"))
    );
    node.set_text_content("Edited").unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    visible.get().unwrap().set(false);
    app.tree();
    let count = wakes.load(Ordering::SeqCst);
    node.set_text_content("Detached").unwrap();
    node.set_attribute("id", "detached").unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), count);
    assert!(app.tree().query("#detached").unwrap().is_none());
}

#[test]
fn invalid_writes_are_atomic_and_class_list_reads_and_toggles_are_live() {
    let app = UiApp::new(|| view! {<p id="target" className="p-2">"Text"</p>});
    let element = app.tree().get_element_by_id("target").unwrap();
    let before = app.tree();
    for (name, value) in [("unknown", "x"), ("className", "p-bad")] {
        assert!(element.set_attribute(name, value).is_err());
        assert_eq!(app.tree(), before);
    }
    let classes = element.class_list();
    for token in ["", "p-2 p-4", "p-bad"] {
        assert!(classes.add(token).is_err());
        assert!(classes.remove(token).is_err());
        assert_eq!(app.tree(), before);
    }
    classes.add("opacity-25").unwrap();
    classes.add("opacity-25").unwrap();
    assert_eq!(classes.tokens(), ["p-2", "opacity-25"]);
    assert!(!classes.toggle("opacity-25").unwrap());
    assert!(classes.toggle("opacity-25").unwrap());
    classes.remove("p-2").unwrap();
    assert_eq!(classes.tokens(), ["opacity-25"]);
}

#[test]
fn tree_handles_and_node_refs_reject_cross_session_reuse() {
    let reference = node_ref();
    let mounted = reference.clone();
    let first = UiApp::new(move || view! {<p id="same" node_ref={mounted}>"First"</p>});
    let second = UiApp::new(|| view! {<p id="same">"Second"</p>});
    let a = first.tree().get_element_by_id("same").unwrap();
    let b = second.tree().get_element_by_id("same").unwrap();
    assert_ne!(a, b);
    assert_eq!(second.tree().check_node(&a), Err(ViewError::ForeignSession));
    assert!(first.tree().check_node(&a).is_ok());
    let third = UiApp::new(move || view! {<p node_ref={reference}>"Third"</p>});
    assert_eq!(
        third.take_errors(),
        ["node ref belongs to another tree session"]
    );
    assert_eq!(a.text_content(), "First");
    let tree = first.tree();
    drop(first);
    assert!(matches!(tree.query("p"), Err(ViewError::SessionDropped)));
    assert_eq!(a.text_content(), "First");
}

#[derive(Clone)]
struct State {
    count: Signal<i32>,
}
#[test]
fn component_state_is_typed_nearest_and_released_with_dynamic_registrations() {
    let visible = Rc::new(Cell::new(None));
    let output = visible.clone();
    let held = Rc::new(RefCell::new(None));
    let inner = held.clone();
    let value = Rc::new(());
    let weak = Rc::downgrade(&value);
    let app = UiApp::new(move || {
        let shown = signal(true);
        output.set(Some(shown));
        let outer = State { count: signal(1) };
        View::element("view")
            .with_state(ComponentState::new(outer))
            .child(move || {
                shown.get().unwrap().then(|| {
                    let state = State { count: signal(2) };
                    *inner.borrow_mut() = Some(state.count);
                    view! {<p id="inner"><span id="leaf">{state.count}</span></p>}
                        .with_state(ComponentState::new(state))
                })
            })
            .child(View::element("p").attr("id", "outer").child("Outer"))
            .with_state(ComponentState::new(value))
    });
    let leaf = app.tree().query("#leaf").unwrap().unwrap();
    assert_eq!(leaf.component_state::<State>().unwrap().count.get(), Ok(2));
    let outer = app.tree().query("#outer").unwrap().unwrap();
    assert_eq!(outer.component_state::<State>().unwrap().count.get(), Ok(1));
    assert!(leaf.component_state::<String>().is_none());
    let count = leaf.component_state::<State>().unwrap().count;
    count.set(4);
    assert_eq!(leaf.text_content(), "4");
    visible.get().unwrap().set(false);
    assert!(leaf.component_state::<State>().is_none());
    assert_eq!(
        held.borrow().unwrap().get(),
        Err(ReactiveError::DisposedSignal)
    );
    drop(app);
    assert!(weak.upgrade().is_none());
}

#[test]
fn active_tree_access_in_real_event_closures_edits_the_next_scene() {
    let app = UiApp::new(|| {
        view! {<view><p id="message">"Original"</p>
        <button onClick={|_| {
            let element=tree().unwrap().query("#message").unwrap().unwrap();
            element.set_text_content("Edited").unwrap();
            element.class_list().add("p-4").unwrap();
        }}>"Edit"</button></view>}
    });
    let before = app.tree();
    assert!(app.dispatch(0));
    let after = app.tree();
    assert_ne!(before, after);
    assert_eq!(
        after.get_element_by_id("message").unwrap().text_content(),
        "Edited"
    );
    assert_eq!(app.patch_passes(), 1);
}

#[path = "../examples/retained_tree.rs"]
mod documented;
#[test]
fn documented_app_runs_its_real_edit_and_authored_change_buttons() {
    let app = UiApp::new(documented::App);
    app.tree();
    app.dispatch(0);
    let edited = app.tree().get_element_by_id("message").unwrap();
    assert_eq!(edited.text_content(), "Hand edit");
    assert!(edited.class_list().contains("opacity-25"));
    app.dispatch(1);
    assert_eq!(edited.text_content(), "Count: 1");
    assert!(edited.class_list().contains("opacity-25"));
}

#[test]
fn text_refs_and_multi_root_state_add_no_layout_nodes() {
    let reference = node_ref();
    let mounted = reference.clone();
    let app = UiApp::new(move || View::text("Text").node_ref(mounted));
    assert!(reference.get().unwrap().as_element().is_none());
    assert_eq!(app.tree().text.as_deref(), Some("Text"));
    assert!(app.tree().children.is_empty());
    let app = UiApp::new(|| {
        View::fragment([
            View::element("p").attr("id", "one"),
            View::element("p").attr("id", "two"),
        ])
        .with_state(ComponentState::new(42usize))
    });
    assert_eq!(app.tree().children.len(), 2);
    for id in ["one", "two"] {
        assert_eq!(
            app.tree()
                .get_element_by_id(id)
                .unwrap()
                .component_state::<usize>(),
            Some(42)
        );
    }
}

#[test]
fn descendant_and_sibling_bindings_preserve_unrelated_ancestor_text_edits() {
    let input = Rc::new(Cell::new(None));
    let output = input.clone();
    let app =
        UiApp::new(move || {
            let count = signal(0);
            output.set(Some(count));
            View::element("view")
                .attr("id", "root")
                .child(View::element("p").attr("id", "edited").child(
                    View::element("span").child(View::live_text(move || count.get().unwrap())),
                ))
                .child(View::element("p").child(View::dynamic(move || count.get().unwrap())))
        });
    let root = app.tree().get_element_by_id("root").unwrap();
    let edited = app.tree().get_element_by_id("edited").unwrap();
    edited.set_text_content("Hand edit").unwrap();
    input.get().unwrap().set(1);
    assert_eq!(edited.text_content(), "Hand edit");
    assert_eq!(
        app.tree()
            .get_element_by_id("edited")
            .unwrap()
            .text_content(),
        "Hand edit"
    );
    root.set_text_content("Root edit").unwrap();
    input.get().unwrap().set(2);
    assert_eq!(root.text_content(), "Root edit");
    assert_eq!(
        app.tree().get_element_by_id("root").unwrap().text_content(),
        "Root edit"
    );
}
#[test]
fn node_ref_clears_after_dynamic_detachment_and_detached_text_does_not_dirty() {
    let reference = node_ref();
    let mounted = reference.clone();
    let input = Rc::new(Cell::new(None));
    let output = input.clone();
    let app = UiApp::new(move || {
        let shown = signal(true);
        output.set(Some(shown));
        view! {<view>{move || shown.get().unwrap().then(||
        view!{<p node_ref={mounted.clone()}>"Child"</p>})}</view>}
    });
    let copied = reference.get().unwrap();
    input.get().unwrap().set(false);
    assert!(reference.get().is_none());
    let before = app.property_patches();
    copied.set_text_content("Detached edit").unwrap();
    assert_eq!(copied.text_content(), "Detached edit");
    assert_eq!(app.property_patches(), before);
    input.get().unwrap().set(true);
    let remounted = reference.get().unwrap();
    assert_ne!(copied, remounted);
    drop(app);
    assert!(reference.get().is_none());
}
#[test]
fn ref_wrapped_non_element_builders_report_misuse() {
    let reference = node_ref();
    let app = UiApp::new(|| View::fragment([]).node_ref(reference).child("bad"));
    assert_eq!(app.take_errors().len(), 1);
    let reference = node_ref();
    let app = UiApp::new(|| View::dynamic(|| "child").node_ref(reference).child("bad"));
    assert_eq!(app.take_errors().len(), 1);
}

#[test]
fn direct_text_binding_reclaims_its_container_but_preserves_outer_edit() {
    let input = Rc::new(Cell::new(None));
    let output = input.clone();
    let app = UiApp::new(move || {
        let count = signal(0);
        output.set(Some(count));
        View::element("view").attr("id", "root").child(
            View::element("p")
                .attr("id", "bound")
                .child(View::live_text(move || count.get().unwrap())),
        )
    });
    let root = app.tree().get_element_by_id("root").unwrap();
    let bound = app.tree().get_element_by_id("bound").unwrap();
    bound.set_text_content("Edit").unwrap();
    input.get().unwrap().set(0);
    assert_eq!(bound.text_content(), "Edit");
    input.get().unwrap().set(1);
    assert_eq!(bound.text_content(), "1");
    root.set_text_content("Outer edit").unwrap();
    input.get().unwrap().set(2);
    assert_eq!(bound.text_content(), "2");
    assert_eq!(root.text_content(), "Outer edit");
}
