//! One checked catalog exposes the same retained resources to every UI host.
use super::tree::{NodeHandle, Record, Tree};
use crate::{HostHandle, HostOp, HostReply, HostType, HostValue, Result};
use std::{cell::RefCell, rc::Rc};

pub(super) fn handle(node: NodeHandle, brand: &str) -> HostValue {
    let scope = node.scope();
    let trace_node = node.clone();
    HostValue::Handle(
        HostHandle::from_shared(brand, node.0)
            .with_trace(scope, move |scope| trace_node.roots(scope)),
    )
}
fn optional(node: Option<NodeHandle>, brand: &str) -> HostValue {
    HostValue::Option(node.map(|node| Box::new(handle(node, brand))))
}
pub(super) fn node(value: &HostValue, tree: &RefCell<Tree>) -> Result<NodeHandle> {
    let HostValue::Handle(handle) = value else {
        return Err("expected retained view node".into());
    };
    // Keep the original allocation: repeated reads preserve handle identity.
    // HostHandle supplies a typed shared clone, never a reconstructed resource.
    let node = NodeHandle(
        handle
            .shared::<RefCell<Record>>()
            .ok_or("invalid retained view node")?,
    );
    if !tree.borrow().owns(&node) {
        return Err("view node belongs to another session".into());
    }
    Ok(node)
}
pub(crate) fn operations(tree: Rc<RefCell<Tree>>) -> Vec<HostOp> {
    let query_tree = tree.clone();
    let mut ops = vec![
        HostOp::new(
            "__view_get_element_by_id",
            vec![HostType::String],
            HostType::Option(Box::new(HostType::Handle("ViewElement".into()))),
            false,
            move |args| {
                let HostValue::String(id) = &args[0] else {
                    return HostReply::Ready(Err("expected element id".into()));
                };
                HostReply::Ready(Ok(optional(
                    query_tree.borrow().element_by_id(id),
                    "ViewElement",
                )))
            },
        )
        .with_namespace_binding("view", "getElementById"),
    ];
    for brand in ["ViewNode", "ViewElement"] {
        let session = tree.clone();
        ops.push(
            HostOp::new(
                &format!("__view_{brand}_parent"),
                vec![HostType::Handle(brand.into())],
                HostType::Option(Box::new(HostType::Handle("ViewNode".into()))),
                false,
                move |args| {
                    HostReply::Ready(
                        node(&args[0], &session).map(|node| optional(node.parent(), "ViewElement")),
                    )
                },
            )
            .with_receiver_property(brand, "parentNode")
            .with_handle_base("ViewElement", "ViewNode"),
        );
        let session = tree.clone();
        ops.push(
            HostOp::new(
                &format!("__view_{brand}_text"),
                vec![HostType::Handle(brand.into())],
                HostType::String,
                false,
                move |args| {
                    HostReply::Ready(
                        node(&args[0], &session).map(|node| HostValue::String(node.text_content())),
                    )
                },
            )
            .with_receiver_property(brand, "textContent"),
        );
    }
    let session = tree.clone();
    ops.push(
        HostOp::new(
            "__view_children",
            vec![HostType::Handle("ViewElement".into())],
            HostType::List(Box::new(HostType::Handle("ViewElement".into()))),
            false,
            move |args| {
                HostReply::Ready(node(&args[0], &session).map(|node| {
                    HostValue::List(
                        node.children()
                            .into_iter()
                            .map(|node| handle(node, "ViewElement"))
                            .collect(),
                    )
                }))
            },
        )
        .with_receiver_property("ViewElement", "children"),
    );
    let session = tree.clone();
    ops.push(
        HostOp::new(
            "__view_attribute",
            vec![HostType::Handle("ViewElement".into()), HostType::String],
            HostType::Option(Box::new(HostType::String)),
            false,
            move |args| {
                HostReply::Ready((|| {
                    let HostValue::String(name) = &args[1] else {
                        return Err("expected attribute name".into());
                    };
                    Ok(HostValue::Option(
                        node(&args[0], &session)?
                            .attribute(name)
                            .map(|value| Box::new(HostValue::String(value))),
                    ))
                })())
            },
        )
        .with_receiver_method("ViewElement", "getAttribute"),
    );
    let session = tree.clone();
    ops.push(
        HostOp::new(
            "__view_classes",
            vec![HostType::Handle("ViewElement".into())],
            HostType::Handle("ViewClassList".into()),
            false,
            move |args| {
                HostReply::Ready(node(&args[0], &session).map(|node| handle(node, "ViewClassList")))
            },
        )
        .with_receiver_property("ViewElement", "classList"),
    );
    ops.extend(super::write_api::operations(tree));
    ops
}

#[cfg(all(test, feature = "compiler"))]
mod tests {
    use super::*;
    use crate::{Hosts, compiler, component::Component};
    #[test]
    fn a_detached_handler_capturing_its_own_node_is_collected() {
        let source = r#"fn child(){let held: Option<ViewElement> = None;
            return (<button id="node" onClick={fn(){held=view.getElementById("node");}}>Keep</button>);}
            export fn App(){let visible=true;return {view:fn(){return (<view>{visible ? child() : None}</view>);},hide:fn(){visible=false;}};}"#;
        let mut app = Component::new(
            compiler::compile_entry(source, &Hosts::default(), "App").unwrap(),
            Hosts::default(),
        )
        .unwrap();
        let first = app.render().unwrap().root;
        let weak = Rc::downgrade(&app.tree.borrow().element_by_id("node").unwrap().0);
        app.event(first.children[0].on_click.unwrap(), vec![])
            .unwrap();
        app.call("hide", vec![]).unwrap();
        app.render().unwrap();
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn source_class_list_retains_a_detached_node_only_until_vm_collection() {
        let source = r#"export fn App(){let visible=true;let held: Option<ViewClassList> = None;
            return {view:fn(){return (<view>{visible ? <p id="node">Hello</p> : None}</view>);},
                save:fn(){held=match(view.getElementById("node")){Some(node)=>Some(node.classList),None=>None};},
                hide:fn(){visible=false;},clear:fn(){held=None;}};}"#;
        let mut app = Component::new(
            compiler::compile_entry(source, &Hosts::default(), "App").unwrap(),
            Hosts::default(),
        )
        .unwrap();
        app.render().unwrap();
        let weak = Rc::downgrade(&app.tree.borrow().element_by_id("node").unwrap().0);
        app.call("save", vec![]).unwrap();
        app.call("hide", vec![]).unwrap();
        app.render().unwrap();
        assert!(weak.upgrade().is_some());
        app.call("clear", vec![]).unwrap();
        app.render().unwrap();
        assert!(weak.upgrade().is_none());
    }
}
