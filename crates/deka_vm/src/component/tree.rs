//! VM consumer of the shared retained native store.
pub(crate) use deka_native_ir::tree::{NodeHandle, Record, Tree};
#[cfg(test)]
use {
    crate::ui::WireNode,
    deka_native_ui::Node,
    std::{cell::RefCell, rc::Rc},
};

#[cfg(test)]
mod tests {
    use super::*;
    fn element(tag: &str, children: Vec<WireNode>) -> WireNode {
        WireNode {
            tag: tag.into(),
            children,
            ..Default::default()
        }
    }
    #[test]
    fn removed_subtrees_are_released_and_a_failed_frame_leaves_the_store_unchanged() {
        let mut tree = Tree::default();
        let first = tree
            .update(element(
                "view",
                vec![element("div", vec![element("p", vec![])])],
            ))
            .unwrap();
        assert_eq!(tree.records.len(), 3);
        let root = tree.root.clone().unwrap();
        let mut invalid = element("view", vec![element("unknown", vec![])]);
        invalid.classes = "p-4".into();
        assert!(tree.update(invalid).is_err());
        assert_eq!(root.snapshot(), first);
        drop(root);
        assert_eq!(tree.records.len(), 3);
        let empty = tree.update(element("view", vec![])).unwrap();
        assert_eq!(empty.id, first.id);
        assert_eq!(tree.records.len(), 1);
        let next = tree.update(element("div", vec![])).unwrap();
        assert_ne!(next.id, first.id);
        assert_eq!(tree.records.len(), 1);
    }
}

#[cfg(all(test, feature = "compiler"))]
mod ownership_tests {
    use super::*;
    use crate::{Hosts, compiler, component::Component};
    #[test]
    fn imperative_values_survive_until_the_last_authored_value_changes() {
        let mut app=Component::new(compiler::compile_entry(r#"export fn App() {
            let count=0; let red=false; let noise=0;
            return {view:fn(){return (<view><p class={red ? "text-[#ff0000]" : "text-[#0000ff]"}>{count % 2}</p></view>);},
                count:fn(){count+=1;}, red:fn(){red=red==false;}, noise:fn(){noise+=1;count+=2;} };
        }"#,&Hosts::default(),"App").unwrap(),Hosts::default()).unwrap();
        app.render().unwrap();
        let paragraph = app.tree.borrow().root.as_ref().unwrap().0.borrow().children[0].clone();
        let leaf = paragraph.0.borrow().children[0].clone();
        let hand_style = WireNode {
            tag: "p".into(),
            classes: "text-[#00ff00]".into(),
            ..Default::default()
        }
        .style()
        .unwrap();
        paragraph.0.borrow_mut().style = hand_style.clone();
        paragraph.0.borrow_mut().classes = "text-[#00ff00]".into();
        leaf.0.borrow_mut().text = Some("Hand edit".into());
        // No VM work is needed for the next snapshot to expose direct Rust edits.
        let before = app.instructions();
        let edited = app.render().unwrap().root;
        assert_eq!(app.instructions(), before);
        assert_eq!(edited.children[0].style, hand_style);
        assert_eq!(
            edited.children[0].children[0].text.as_deref(),
            Some("Hand edit")
        );
        app.call("noise", vec![]).unwrap();
        let unchanged = app.render().unwrap().root;
        assert_eq!(unchanged.children[0].style, hand_style);
        assert_eq!(
            unchanged.children[0].children[0].text.as_deref(),
            Some("Hand edit")
        );
        app.call("count", vec![]).unwrap();
        let changed = app.render().unwrap().root;
        assert_eq!(changed.children[0].children[0].text.as_deref(), Some("1"));
        assert_eq!(changed.children[0].style, hand_style);
        app.call("red", vec![]).unwrap();
        let changed = app.render().unwrap().root;
        assert_ne!(changed.children[0].style, hand_style);
        assert_eq!(changed.children[0].children[0].text.as_deref(), Some("1"));
    }
}

#[cfg(test)]
mod handle_tests {
    use super::*;
    fn source() -> WireNode {
        WireNode {
            tag: "view".into(),
            children: vec![WireNode {
                tag: "p".into(),
                children: vec![WireNode {
                    text: Some("Hello".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn handles_retain_one_node_and_its_subtree_without_retaining_its_parent() {
        let mut tree = Tree::default();
        tree.update(source()).unwrap();
        let root = tree.root.clone().unwrap();
        let node = root.0.borrow().children[0].clone();
        let alias = tree.lookup(node.0.borrow().id).unwrap();
        assert!(node == alias);
        let weak = Rc::downgrade(&node.0);
        let child = Rc::downgrade(&node.0.borrow().children[0].0);
        let parent = Rc::downgrade(&root.0);
        tree.update(WireNode {
            tag: "view".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(node.0.borrow().parent.is_none());
        assert_eq!(node.snapshot().children[0].text.as_deref(), Some("Hello"));
        assert!(tree.lookup(node.0.borrow().id).unwrap() == node);
        tree.update(source()).unwrap();
        assert!(tree.root.as_ref().unwrap().0.borrow().children[0] != node);
        drop(alias);
        drop(root);
        drop(tree);
        assert!(parent.upgrade().is_none());
        assert!(weak.upgrade().is_some());
        assert!(child.upgrade().is_some());
        drop(node);
        assert!(weak.upgrade().is_none());
        assert!(child.upgrade().is_none());
    }
    #[test]
    fn sessions_never_alias_and_weak_indices_release_removed_nodes() {
        let mut first = Tree::default();
        let mut second = Tree::default();
        first.update(source()).unwrap();
        second.update(source()).unwrap();
        let a = first.root.clone().unwrap();
        let b = second.root.clone().unwrap();
        assert_eq!(a.snapshot().id, b.snapshot().id);
        assert!(a != b);
        let old = Rc::downgrade(&a.0.borrow().children[0].0);
        for _ in 0..100 {
            first
                .update(WireNode {
                    tag: "view".into(),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(first.records.len(), 1);
            first.update(source()).unwrap();
            assert_eq!(first.records.len(), 3);
        }
        assert!(old.upgrade().is_none());
        drop(first);
        assert!(
            a.0.borrow().children[0]
                .0
                .borrow()
                .parent
                .as_ref()
                .unwrap()
                .upgrade()
                .is_some()
        );
        let child = a.0.borrow().children[0].clone();
        let oldroot = Rc::downgrade(&a.0);
        drop(a);
        assert!(oldroot.upgrade().is_none());
        assert!(
            child
                .0
                .borrow()
                .parent
                .as_ref()
                .unwrap()
                .upgrade()
                .is_none()
        );
        assert_eq!(child.snapshot().children[0].text.as_deref(), Some("Hello"));
    }
}

#[cfg(all(test, feature = "compiler"))]
mod typed_node_tests {
    use super::*;
    use crate::{
        HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, compiler, component::Component,
    };
    const SOURCE: &str = r#"import {nodeText} from "vm:host";
        export fn App(){
            let visible=true; let title="Hello"; let held: Option<TestNode> = None;
            return {view:fn(){return (<view>{visible ? <p>{title}</p> : None}</view>);},
                retain:fn(node:TestNode){held=Some(node);},
                read:fn(){return match(held){Some(node)=>node.snapshotText(),None=>"Empty"};},
                rename:fn(){title="Changed";}, forget:fn(){held=None;}, hide:fn(){visible=false;} };
        }"#;
    fn catalog() -> Hosts {
        let mut hosts = Hosts::default();
        hosts
            .register(
                HostOp::new(
                    "nodeText",
                    vec![HostType::Handle("TestNode".into())],
                    HostType::String,
                    false,
                    |args| {
                        let HostValue::Handle(handle) = &args[0] else {
                            return HostReply::Ready(Err("expected node handle".into()));
                        };
                        let Some(record) = handle.downcast_ref::<RefCell<Record>>() else {
                            return HostReply::Ready(Err("invalid node resource".into()));
                        };
                        let snapshot = NodeHandle::snapshot_record(&record.borrow());
                        fn text(node: &Node) -> String {
                            node.text.clone().unwrap_or_default()
                                + &node.children.iter().map(text).collect::<String>()
                        }
                        HostReply::Ready(Ok(HostValue::String(text(&snapshot))))
                    },
                )
                .with_receiver_method("TestNode", "snapshotText"),
            )
            .unwrap();
        hosts
            .register(HostOp::new(
                "otherNode",
                vec![],
                HostType::Handle("OtherNode".into()),
                false,
                |_| HostReply::Ready(Ok(HostValue::Handle(HostHandle::new("OtherNode", 0u8)))),
            ))
            .unwrap();
        hosts
    }
    #[test]
    fn a_typed_vm_alias_retains_a_real_detached_node_until_collection() {
        let hosts = catalog();
        let mut app = Component::new(
            compiler::compile_entry(SOURCE, &hosts, "App").unwrap(),
            hosts,
        )
        .unwrap();
        app.render().unwrap();
        let node = app.tree.borrow().root.as_ref().unwrap().0.borrow().children[0].clone();
        let id = node.0.borrow().id;
        let weak = Rc::downgrade(&node.0);
        let a = HostHandle::from_shared("TestNode", node.0.clone());
        let b = HostHandle::from_shared("TestNode", app.tree.borrow().lookup(id).unwrap().0);
        assert_eq!(a, b); // Repeated lookups name one allocation, not proxy wrappers.
        app.call("retain", vec![HostValue::Handle(a)]).unwrap();
        drop(b);
        drop(node);
        app.call("rename", vec![]).unwrap();
        app.render().unwrap();
        let current = app.tree.borrow().root.as_ref().unwrap().0.borrow().children[0].clone();
        assert!(Rc::ptr_eq(&weak.upgrade().unwrap(), &current.0));
        drop(current);
        assert_eq!(
            app.call("read", vec![]).unwrap(),
            HostValue::String("Changed".into())
        );
        app.call("hide", vec![]).unwrap();
        app.render().unwrap();
        assert!(weak.upgrade().is_some());
        assert_eq!(
            app.call("read", vec![]).unwrap(),
            HostValue::String("Changed".into())
        );
        app.call("forget", vec![]).unwrap();
        app.render().unwrap();
        assert!(weak.upgrade().is_none());
        assert!(app.tree.borrow().lookup(id).is_none());
    }
    #[test]
    fn node_values_cannot_be_forged_as_records_or_replaced_with_other_brands() {
        let hosts = catalog();
        for value in ["\"fake\"", "{value:1}", "1", "otherNode()"] {
            let source = SOURCE
                .replace("{nodeText}", "{nodeText, otherNode}")
                .replace("held=Some(node);", &format!("held=Some({value});"));
            let error = compiler::compile_entry(&source, &hosts, "App").unwrap_err();
            assert!(error.contains("TestNode"), "{error}");
        }
    }
}
