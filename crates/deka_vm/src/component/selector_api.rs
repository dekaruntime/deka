//! Checked source queries share the retained tree's backend-independent matcher.
use super::{read_api::handle, selectors::Selector, tree::Tree};
use crate::{HostOp, HostReply, HostType, HostValue};
use std::{cell::RefCell, rc::Rc};

pub(super) fn operations(tree: Rc<RefCell<Tree>>) -> Vec<HostOp> {
    [false, true]
        .into_iter()
        .map(|all| {
            let tree = tree.clone();
            let output = if all {
                HostType::List(Box::new(HostType::Handle("ViewElement".into())))
            } else {
                HostType::Option(Box::new(HostType::Handle("ViewElement".into())))
            };
            HostOp::new(
                if all {
                    "__view_query_all"
                } else {
                    "__view_query_first"
                },
                vec![HostType::String],
                output,
                false,
                move |args| {
                    HostReply::Ready((|| {
                        let HostValue::String(source) = &args[0] else {
                            return Err("expected selector string".into());
                        };
                        let selector = Selector::parse(source)?;
                        let nodes = tree.borrow().query(&selector, all);
                        Ok(if all {
                            HostValue::List(
                                nodes
                                    .into_iter()
                                    .map(|node| handle(node, "ViewElement"))
                                    .collect(),
                            )
                        } else {
                            HostValue::Option(
                                nodes
                                    .into_iter()
                                    .next()
                                    .map(|node| Box::new(handle(node, "ViewElement"))),
                            )
                        })
                    })())
                },
            )
            .with_result_channel()
            .with_namespace_binding(
                "view",
                if all {
                    "querySelectorAll"
                } else {
                    "querySelector"
                },
            )
        })
        .collect()
}
