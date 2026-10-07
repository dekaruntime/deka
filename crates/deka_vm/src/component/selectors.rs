//! VM selector consumer; the matcher has one shared implementation.
pub(super) use deka_native_ir::selectors::*;

#[cfg(test)]
mod tests {
    use super::*;

    struct TestNode {
        tag: Option<&'static str>,
        id: Option<&'static str>,
        classes: &'static str,
        parent: Option<usize>,
        children: Vec<usize>,
    }
    struct TestTree(Vec<TestNode>);
    impl SelectorTree for TestTree {
        type Node = usize;
        fn with_element<R>(
            &self,
            node: &usize,
            read: impl for<'a> FnOnce(Option<Element<'a>>) -> R,
        ) -> R {
            let node = &self.0[*node];
            read(node.tag.map(|tag| Element {
                tag,
                id: node.id,
                classes: node.classes,
            }))
        }
        fn parent(&self, node: &usize) -> Option<usize> {
            self.0[*node].parent
        }
        fn children(&self, node: &usize) -> Vec<usize> {
            self.0[*node].children.clone()
        }
    }
    fn tree() -> TestTree {
        TestTree(vec![
            TestNode {
                tag: Some("view"),
                id: Some("root"),
                classes: "app",
                parent: None,
                children: vec![1, 5, 9],
            },
            TestNode {
                tag: Some("div"),
                id: Some("card1"),
                classes: "card focused",
                parent: Some(0),
                children: vec![2, 3, 4],
            },
            TestNode {
                tag: None,
                id: None,
                classes: "action",
                parent: Some(1),
                children: vec![],
            },
            TestNode {
                tag: Some("p"),
                id: None,
                classes: "title",
                parent: Some(1),
                children: vec![],
            },
            TestNode {
                tag: Some("button"),
                id: Some("save"),
                classes: "action focused",
                parent: Some(1),
                children: vec![],
            },
            TestNode {
                tag: Some("div"),
                id: Some("card2"),
                classes: "card",
                parent: Some(0),
                children: vec![6],
            },
            TestNode {
                tag: Some("section"),
                id: None,
                classes: "panel",
                parent: Some(5),
                children: vec![7],
            },
            TestNode {
                tag: Some("div"),
                id: Some("nested"),
                classes: "card",
                parent: Some(6),
                children: vec![8],
            },
            TestNode {
                tag: Some("button"),
                id: Some("later"),
                classes: "action",
                parent: Some(7),
                children: vec![],
            },
            TestNode {
                tag: None,
                id: None,
                classes: "",
                parent: Some(0),
                children: vec![],
            },
        ])
    }
    fn query(source: &str) -> Vec<usize> {
        Selector::parse(source).unwrap().query_all(&tree(), &0)
    }
    #[test]
    fn grammar_accepts_only_complete_supported_selectors() {
        for source in [
            "view",
            "#root",
            ".card",
            "div#card1.card.focused",
            "view > div.card .action",
            "\n view\t>\n.card \r.action ",
            "#café",
            ".-mt-2",
            ".--custom",
        ] {
            assert!(Selector::parse(source).is_ok(), "{source:?}");
        }
        for source in [
            "",
            " ",
            "> div",
            "div >",
            "div >> span",
            "div + span",
            "div ~ span",
            "[id]",
            ".card:hover",
            "#x, .card",
            "*",
            "div/#x",
            "#",
            "div.",
            ".1x",
            "-",
            "-2",
            "\\div",
            ".bg-[red]",
        ] {
            let error = Selector::parse(source).unwrap_err();
            assert!(
                error.starts_with("invalid selector at byte "),
                "{source:?}: {error}"
            );
        }
    }
    #[test]
    fn compound_queries_use_exact_ids_classes_and_element_kinds() {
        assert_eq!(query("div#card1.card.focused"), [1]);
        assert_eq!(query("#save#save"), [4]);
        assert!(query("#save#later").is_empty());
        assert!(query(".car").is_empty());
        assert_eq!(query(".action"), [4, 8]);
        assert_eq!(query("view.app#root"), [0]);
    }
    #[test]
    fn child_and_descendant_relations_preserve_tree_order_and_backtrack() {
        assert_eq!(query("#card2 .action"), [8]);
        assert!(query("#card2 > .action").is_empty());
        // The nearest .card ancestor of node 8 is not a child of view. The
        // outer .card is, so matching must backtrack instead of failing early.
        assert_eq!(query("view > .card .action"), [4, 8]);
        assert_eq!(query(".panel > .card > button"), [8]);
        assert_eq!(
            Selector::parse(".action").unwrap().query_first(&tree(), &0),
            Some(4)
        );
        assert_eq!(
            Selector::parse(".missing")
                .unwrap()
                .query_first(&tree(), &0),
            None
        );
    }
}
