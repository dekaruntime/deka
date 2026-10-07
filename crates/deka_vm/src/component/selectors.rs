//! The approved native selector subset; no browser or unsupported-syntax fallback.

pub(super) struct Element<'a> {
    pub tag: &'a str,
    pub id: Option<&'a str>,
    pub classes: &'a str,
}

/// The reader runs under the adapter's borrow, so retained nodes need not copy
/// tag/attribute strings or expose their private allocation to the matcher.
pub(super) trait SelectorTree {
    type Node: Clone + Eq;

    fn with_element<R>(
        &self,
        node: &Self::Node,
        read: impl for<'a> FnOnce(Option<Element<'a>>) -> R,
    ) -> R;
    fn parent(&self, node: &Self::Node) -> Option<Self::Node>;
    fn children(&self, node: &Self::Node) -> Vec<Self::Node>;
}

pub(super) fn class_tokens(classes: &str) -> impl Iterator<Item = &str> {
    classes.split_whitespace()
}

#[derive(Debug, PartialEq, Eq)]
struct Compound {
    tag: Option<String>,
    ids: Vec<String>,
    classes: Vec<String>,
}
impl Compound {
    fn matches<T: SelectorTree>(&self, tree: &T, node: &T::Node) -> bool {
        tree.with_element(node, |element| {
            let Some(element) = element else {
                return false;
            };
            self.tag.as_deref().is_none_or(|tag| element.tag == tag)
                && self.ids.iter().all(|id| element.id == Some(id.as_str()))
                && self
                    .classes
                    .iter()
                    .all(|class| class_tokens(element.classes).any(|token| token == class))
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Relation {
    Descendant,
    Child,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Selector {
    compounds: Vec<Compound>,
    relations: Vec<Relation>,
}
impl Selector {
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut parser = Parser { source, offset: 0 };
        parser.whitespace();
        let mut compounds = vec![parser.compound()?];
        let mut relations = Vec::new();
        loop {
            let space = parser.whitespace();
            if parser.peek().is_none() {
                break;
            }
            let relation = if parser.peek() == Some('>') {
                parser.advance();
                parser.whitespace();
                Relation::Child
            } else if space {
                Relation::Descendant
            } else {
                return Err(parser.error("unsupported selector syntax"));
            };
            compounds.push(parser.compound()?);
            relations.push(relation);
        }
        Ok(Self {
            compounds,
            relations,
        })
    }

    /// `view` is the document-equivalent: its supplied live element root is
    /// part of the query. Preorder follows authored/imperative child order.
    pub fn query_all<T: SelectorTree>(&self, tree: &T, root: &T::Node) -> Vec<T::Node> {
        self.query(tree, root, false)
    }
    pub fn query_first<T: SelectorTree>(&self, tree: &T, root: &T::Node) -> Option<T::Node> {
        self.query(tree, root, true).into_iter().next()
    }
    fn query<T: SelectorTree>(&self, tree: &T, root: &T::Node, first_only: bool) -> Vec<T::Node> {
        let mut pending = vec![root.clone()];
        let mut matches = Vec::new();
        while let Some(node) = pending.pop() {
            if self.matches(tree, &node) {
                matches.push(node.clone());
                if first_only {
                    break;
                }
            }
            pending.extend(tree.children(&node).into_iter().rev());
        }
        matches
    }
    fn matches<T: SelectorTree>(&self, tree: &T, node: &T::Node) -> bool {
        let last = self.compounds.len() - 1;
        if !self.compounds[last].matches(tree, node) {
            return false;
        }
        let mut pending = vec![(last, node.clone())];
        let mut visited = Vec::new();
        while let Some((index, node)) = pending.pop() {
            if index == 0 {
                return true;
            }
            if visited.contains(&(index, node.clone())) {
                continue;
            }
            visited.push((index, node.clone()));
            let mut parent = tree.parent(&node);
            while let Some(ancestor) = parent {
                if self.compounds[index - 1].matches(tree, &ancestor) {
                    pending.push((index - 1, ancestor.clone()));
                }
                if self.relations[index - 1] == Relation::Child {
                    break;
                }
                parent = tree.parent(&ancestor);
            }
        }
        false
    }
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
}
impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }
    fn advance(&mut self) {
        if let Some(character) = self.peek() {
            self.offset += character.len_utf8();
        }
    }
    fn whitespace(&mut self) -> bool {
        let start = self.offset;
        while self.peek().is_some_and(char::is_whitespace) {
            self.advance();
        }
        self.offset != start
    }
    fn error(&self, message: &str) -> String {
        format!("invalid selector at byte {}: {message}", self.offset)
    }
    fn compound(&mut self) -> Result<Compound, String> {
        let mut compound = Compound {
            tag: None,
            ids: Vec::new(),
            classes: Vec::new(),
        };
        if self.peek().is_some_and(identifier_start) {
            compound.tag = Some(self.identifier()?);
        }
        while let Some(prefix @ ('#' | '.')) = self.peek() {
            self.advance();
            let name = self.identifier()?;
            if prefix == '#' {
                compound.ids.push(name);
            } else {
                compound.classes.push(name);
            }
        }
        if compound.tag.is_none() && compound.ids.is_empty() && compound.classes.is_empty() {
            return Err(self.error("expected a tag, #id or .class"));
        }
        Ok(compound)
    }
    fn identifier(&mut self) -> Result<String, String> {
        let start = self.offset;
        let Some(first) = self.peek().filter(|character| identifier_start(*character)) else {
            return Err(self.error("expected an unescaped identifier"));
        };
        self.advance();
        if first == '-' && !self.peek().is_some_and(identifier_start) {
            return Err(self.error("a leading hyphen needs an identifier character"));
        }
        while self.peek().is_some_and(identifier_continue) {
            self.advance();
        }
        Ok(self.source[start..self.offset].to_owned())
    }
}
fn identifier_start(character: char) -> bool {
    character.is_ascii_alphabetic()
        || matches!(character, '_' | '-')
        || (!character.is_ascii() && !character.is_whitespace())
}
fn identifier_continue(character: char) -> bool {
    identifier_start(character) || character.is_ascii_digit()
}

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
