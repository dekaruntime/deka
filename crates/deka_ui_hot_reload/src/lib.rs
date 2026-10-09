//! Development-only view templates. Rust expressions remain compiled opaque slots.
pub mod files;
use proc_macro2::TokenStream;
use quote::ToTokens;
use rstml::node::{Node, NodeAttribute};
use std::collections::BTreeMap;
use syn::{Expr, Lit, visit_mut::VisitMut};

/// Typed applications and the supervisor share the fallback notification.
pub const RESTART_PREFIX: &str = "deka dev: rebuild required: ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Element {
        tag: String,
        attributes: BTreeMap<String, String>,
        bindings: BTreeMap<String, String>,
    },
    Text(String),
    Hole(String),
    Component {
        code: String,
        props: BTreeMap<String, Literal>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Literal {
    pub kind: String,
    pub value: String,
}
pub fn literal(expr: &Expr) -> Option<Literal> {
    let expr = value(expr);
    let (expr, negative) = match expr {
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => (value(&unary.expr), true),
        _ => (expr, false),
    };
    let Expr::Lit(expr) = expr else { return None };
    let (kind, value) = match &expr.lit {
        Lit::Str(s) if !negative => ("string".into(), s.value()),
        Lit::Char(c) if !negative => ("char".into(), c.value().to_string()),
        Lit::Bool(b) if !negative => ("bool".into(), b.value.to_string()),
        Lit::Int(i) => (
            format!("int:{}", i.suffix()),
            format!("{}{}", if negative { "-" } else { "" }, i.base10_digits()),
        ),
        Lit::Float(f) => (
            format!("float:{}", f.suffix()),
            format!("{}{}", if negative { "-" } else { "" }, f.base10_digits()),
        ),
        _ => return None,
    };
    Some(Literal { kind, value })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateNode {
    pub id: usize,
    pub kind: Kind,
    pub children: Vec<Self>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub nodes: Vec<TemplateNode>,
}
fn value(mut expr: &Expr) -> &Expr {
    while let Expr::Block(block) = expr {
        if block.attrs.is_empty()
            && block.label.is_none()
            && let [syn::Stmt::Expr(inner, None)] = block.block.stmts.as_slice()
        {
            expr = inner;
        } else {
            break;
        }
    }
    expr
}
/// Shared by the compiler and runtime: IDs follow the expanded view's preorder.
pub fn parse(tokens: TokenStream) -> Result<Template, String> {
    let mut next = 0;
    Ok(Template {
        nodes: convert(
            &rstml::parse2(tokens).map_err(|e| e.to_string())?,
            &mut next,
        )?,
    })
}
fn convert(nodes: &[Node], next: &mut usize) -> Result<Vec<TemplateNode>, String> {
    let mut output = vec![];
    for node in nodes {
        match node {
            Node::Comment(_) => (),
            Node::Fragment(fragment) => output.extend(convert(&fragment.children, next)?),
            Node::Text(text) => {
                for part in text_parts(&text.value.value())? {
                    let id = *next;
                    *next += 1;
                    output.push(TemplateNode {
                        id,
                        kind: part,
                        children: vec![],
                    });
                }
            }
            Node::Block(block) => {
                if block.try_block().is_none() {
                    return Err("invalid Rust child".into());
                }
                let id = *next;
                *next += 1;
                output.push(TemplateNode {
                    id,
                    kind: Kind::Hole(block.to_token_stream().to_string()),
                    children: vec![],
                });
            }
            Node::Element(element) => {
                let id = *next;
                *next += 1;
                let tag = element.name().to_string();
                if tag
                    .rsplit("::")
                    .next()
                    .is_some_and(|s| s.trim().starts_with(char::is_uppercase))
                {
                    let mut normalized = element.clone();
                    let mut props = BTreeMap::new();
                    for attribute in normalized.attributes_mut() {
                        let NodeAttribute::Attribute(attribute) = attribute else {
                            return Err("use a named component prop".into());
                        };
                        let key = attribute.key.to_string();
                        if let Some(value) = attribute.value().and_then(literal) {
                            if props.insert(key, value.clone()).is_some() {
                                return Err("duplicate component prop".into());
                            }
                            let marker = format!("__deka_literal_{}", value.kind.replace(':', "_"));
                            if let rstml::node::KeyedAttributeValue::Value(expr) =
                                &mut attribute.possible_value
                            {
                                expr.value = rstml::node::KVAttributeValue::Expr(
                                    syn::parse_str(&marker).map_err(|e| e.to_string())?,
                                );
                            }
                        }
                    }
                    output.push(TemplateNode {
                        id,
                        kind: Kind::Component {
                            code: normalized.to_token_stream().to_string(),
                            props,
                        },
                        children: vec![],
                    });
                    continue;
                }
                let mut attributes = BTreeMap::new();
                let mut bindings = BTreeMap::new();
                for attribute in element.attributes() {
                    let NodeAttribute::Attribute(attribute) = attribute else {
                        return Err("use a named attribute".into());
                    };
                    let key = attribute.key.to_string();
                    if attributes.contains_key(&key) || bindings.contains_key(&key) {
                        return Err("duplicate attribute".into());
                    }
                    let expr = value(attribute.value().ok_or("attribute requires a Rust value")?);
                    if deka_native_ir::is_supported_attribute(&key)
                        && let Expr::Lit(lit) = expr
                    {
                        let literal = match &lit.lit {
                            Lit::Str(s) => s.value(),
                            Lit::Bool(b) => b.value.to_string(),
                            Lit::Int(i) => i.base10_digits().into(),
                            Lit::Float(f) => f.base10_digits().into(),
                            _ => {
                                bindings.insert(key, expr.to_token_stream().to_string());
                                continue;
                            }
                        };
                        attributes.insert(key, literal);
                    } else if deka_native_ir::is_supported_attribute(&key)
                        || matches!(
                            key.as_str(),
                            "value"
                                | "node_ref"
                                | "onClick"
                                | "onInput"
                                | "onKeyDown"
                                | "onContextMenu"
                        )
                        || key.starts_with("class:")
                    {
                        if let Some(class) = key.strip_prefix("class:") {
                            deka_native_ir::apply_classes(&mut Default::default(), class)?;
                        }
                        bindings.insert(key, expr.to_token_stream().to_string());
                    } else {
                        return Err(format!("unsupported native attribute {key}"));
                    }
                }
                deka_native_ir::WireNode {
                    tag: tag.clone(),
                    classes: attributes.get("class").cloned().unwrap_or_default(),
                    attributes: attributes.clone(),
                    ..Default::default()
                }
                .style()?;
                output.push(TemplateNode {
                    id,
                    kind: Kind::Element {
                        tag,
                        attributes,
                        bindings,
                    },
                    children: convert(&element.children, next)?,
                });
            }
            _ => return Err("text must be quoted; Rust children go inside braces".into()),
        }
    }
    Ok(output)
}
pub fn text_parts(text: &str) -> Result<Vec<Kind>, String> {
    let mut parts = vec![];
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                plain.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                plain.push('}');
            }
            '{' => {
                if !plain.is_empty() {
                    parts.push(Kind::Text(std::mem::take(&mut plain)));
                }
                let mut name = String::new();
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == '}' {
                        closed = true;
                        break;
                    }
                    name.push(ch);
                }
                if !closed {
                    return Err("unclosed text interpolation".into());
                }
                syn::parse_str::<syn::Ident>(&name)
                    .map_err(|_| "text interpolation requires an identifier")?;
                parts.push(Kind::Hole(format!("interpolation:{name}")));
            }
            '}' => return Err("unmatched text interpolation brace".into()),
            _ => plain.push(ch),
        }
    }
    if !plain.is_empty() || parts.is_empty() {
        parts.push(Kind::Text(plain));
    }
    Ok(parts)
}
impl Template {
    pub fn flattened(&self) -> Vec<&TemplateNode> {
        fn visit<'a>(nodes: &'a [TemplateNode], out: &mut Vec<&'a TemplateNode>) {
            for node in nodes {
                out.push(node);
                visit(&node.children, out);
            }
        }
        let mut out = vec![];
        visit(&self.nodes, &mut out);
        out
    }
    /// Match compiled slots by expression and occurrence; static siblings by
    /// explicit id first, then compatible position. Never invent Rust bindings.
    pub fn map_from(&self, old: &Self) -> Result<BTreeMap<usize, usize>, String> {
        let old_nodes = old.flattened();
        let new_nodes = self.flattened();
        let compiled = |node: &TemplateNode| match &node.kind {
            Kind::Hole(code) => Some(format!("hole:{code}")),
            Kind::Component { code, .. } => Some(format!("component:{code}")),
            Kind::Element { tag, bindings, .. } if !bindings.is_empty() => {
                Some(format!("bindings:{tag}:{bindings:?}"))
            }
            _ => None,
        };
        let old_compiled: Vec<_> = old_nodes.iter().filter_map(|n| compiled(n)).collect();
        let new_compiled: Vec<_> = new_nodes.iter().filter_map(|n| compiled(n)).collect();
        let mut old_sorted = old_compiled.clone();
        old_sorted.sort();
        let mut new_sorted = new_compiled.clone();
        new_sorted.sort();
        if old_sorted != new_sorted {
            return Err("compiled expressions or component props changed".into());
        }
        // A permutation of literal values on otherwise identical calls cannot
        // distinguish moving instances from editing their props without ids.
        for key in old_compiled
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
        {
            let props = |nodes: &[&TemplateNode]| {
                nodes
                    .iter()
                    .filter_map(|node| match &node.kind {
                        Kind::Component { props, .. } if compiled(node).as_ref() == Some(key) => {
                            Some(props.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let before = props(&old_nodes);
            let after = props(&new_nodes);
            if before.len() < 2 || before == after {
                continue;
            }
            let keyed = |values: &[BTreeMap<String, Literal>]| {
                values.iter().enumerate().all(|(index, props)| {
                    props.get("id").is_some_and(|id| {
                        values[..index]
                            .iter()
                            .all(|other| other.get("id") != Some(id))
                    })
                })
            };
            if keyed(&before) && keyed(&after) {
                continue;
            }
            let mut unmatched = before;
            for value in after {
                if let Some(index) = unmatched.iter().position(|old| *old == value) {
                    unmatched.remove(index);
                } else {
                    break;
                }
            }
            if unmatched.is_empty() {
                return Err(
                    "reordering identical component calls needs distinct component ids; rebuilding"
                        .into(),
                );
            }
        }
        let mut result = BTreeMap::new();
        let mut used = std::collections::BTreeSet::new();
        for new in &new_nodes {
            if let Some(key) = compiled(new) {
                let candidates: Vec<_> = old_nodes
                    .iter()
                    .filter(|n| !used.contains(&n.id) && compiled(n).as_ref() == Some(&key))
                    .collect();
                let id = match &new.kind {
                    Kind::Component { props, .. } => props.get("id"),
                    _ => None,
                };
                let previous = candidates.iter().copied().find(|n| matches!(&n.kind, Kind::Component { props, .. } if id.is_some() && props.get("id") == id))
                    .or_else(|| candidates.first().copied())
                    .ok_or("compiled slot missing")?;
                result.insert(new.id, previous.id);
                used.insert(previous.id);
            }
        }
        for new in &new_nodes {
            if result.contains_key(&new.id) {
                continue;
            }
            if let Kind::Element {attributes,bindings,..}=&new.kind
                && bindings.is_empty()
                && let Some(key)=attributes.get("id")
                && let Some(previous)=old_nodes.iter().find(|old| !used.contains(&old.id) && matches!(&old.kind,Kind::Element {attributes,bindings,..} if bindings.is_empty() && attributes.get("id")==Some(key))) {
                result.insert(new.id,previous.id);used.insert(previous.id);
            }
        }
        for (new, previous) in self.nodes.iter().zip(&old.nodes) {
            if matches!(&new.kind,Kind::Element {bindings,..} if bindings.is_empty())
                && matches!(&previous.kind,Kind::Element {bindings,..} if bindings.is_empty())
                && !result.contains_key(&new.id)
                && !used.contains(&previous.id)
            {
                result.insert(new.id, previous.id);
                used.insert(previous.id);
            }
        }
        // Surrounding elements inherit identity from their compiled direct
        // children before positional matching can spend those allocations on
        // freshly inserted static siblings.
        fn parent_map(nodes: &[TemplateNode], out: &mut BTreeMap<usize, usize>) {
            for node in nodes {
                for child in &node.children {
                    out.insert(child.id, node.id);
                }
                parent_map(&node.children, out);
            }
        }
        let mut old_parent = BTreeMap::new();
        parent_map(&old.nodes, &mut old_parent);
        for new in &new_nodes {
            if result.contains_key(&new.id) {
                continue;
            }
            if !matches!(&new.kind,Kind::Element {bindings,..} if bindings.is_empty()) {
                continue;
            }
            let candidates: std::collections::BTreeSet<_> = new
                .children
                .iter()
                .filter_map(|child| {
                    result
                        .get(&child.id)
                        .and_then(|id| old_parent.get(id))
                        .copied()
                })
                .collect();
            if candidates.len() == 1 {
                let candidate = *candidates.first().unwrap();
                if !used.contains(&candidate) && old_nodes.iter().any(|node| {
                    node.id == candidate
                        && matches!(&node.kind,Kind::Element {bindings,..} if bindings.is_empty())
                }) {
                    result.insert(new.id, candidate);
                    used.insert(candidate);
                }
            }
        }
        for new in new_nodes {
            if result.contains_key(&new.id) {
                continue;
            }
            let compatible = |old: &&TemplateNode| {
                !used.contains(&old.id)
                    && match (&new.kind, &old.kind) {
                        (Kind::Text(_), Kind::Text(_)) => true,
                        (
                            Kind::Element {
                                bindings: a_bind, ..
                            },
                            Kind::Element {
                                bindings: b_bind, ..
                            },
                        ) => a_bind.is_empty() && b_bind.is_empty(),
                        _ => false,
                    }
            };
            let keyed = if let Kind::Element { attributes, .. } = &new.kind {
                attributes.get("id").and_then(|key| old_nodes.iter().filter(|old|compatible(old)).find(|old| matches!(&old.kind,Kind::Element {attributes,..} if attributes.get("id")==Some(key))))
            } else {
                None
            };
            let previous = keyed
                .or_else(|| {
                    old_nodes
                        .iter()
                        .filter(|old| compatible(old))
                        .find(|old| old.id == new.id)
                })
                .or_else(|| old_nodes.iter().find(|old| compatible(old)));
            if let Some(previous) = previous {
                result.insert(new.id, previous.id);
                used.insert(previous.id);
            }
        }
        // Occurrence matching cannot identify identical, unkeyed compiled
        // slots moved between parents. Refuse that save before planning edits.
        let mut new_parent = BTreeMap::new();
        parent_map(&self.nodes, &mut new_parent);
        for node in self.flattened() {
            let Some(key) = compiled(node) else { continue };
            if old_compiled.iter().filter(|old| **old == key).count() < 2 {
                continue;
            }
            if let Kind::Component { props, .. } = &node.kind
                && let Some(id) = props.get("id")
            {
                let same_id = |other: &TemplateNode| {
                    compiled(other).as_ref() == Some(&key)
                        && matches!(&other.kind, Kind::Component { props, .. } if props.get("id") == Some(id))
                };
                if old_nodes.iter().filter(|node| same_id(node)).count() == 1
                    && self.flattened().iter().filter(|node| same_id(node)).count() == 1
                {
                    continue;
                }
            }
            let previous = result[&node.id];
            let parent = new_parent
                .get(&node.id)
                .and_then(|parent| result.get(parent));
            if parent != old_parent.get(&previous) {
                return Err(
                    "moving identical compiled slots needs distinct component ids; rebuilding"
                        .into(),
                );
            }
        }
        for node in self.flattened() {
            if let Kind::Element {
                attributes,
                bindings,
                ..
            } = &node.kind
                && bindings.keys().any(|key| key.starts_with("class:"))
                && let Some(previous) = result
                    .get(&node.id)
                    .and_then(|id| old_nodes.iter().find(|old| old.id == *id))
                && let Kind::Element {
                    attributes: old_attributes,
                    ..
                } = &previous.kind
                && attributes.get("class") != old_attributes.get("class")
            {
                return Err(
                    "changing a class base with compiled toggles requires rebuilding".into(),
                );
            }
        }
        for (new, previous) in self.nodes.iter().zip(&old.nodes) {
            if std::mem::discriminant(&new.kind) != std::mem::discriminant(&previous.kind) {
                return Err("changing template root kind requires rebuilding".into());
            }
        }
        if self.nodes.len() != old.nodes.len() {
            return Err("changing template root count requires rebuilding".into());
        }
        Ok(result)
    }
}
#[derive(Clone, Debug)]
pub struct Source {
    pub outside: String,
    pub templates: Vec<Template>,
    pub locations: Vec<(usize, usize)>,
}
impl Source {
    pub fn parse(source: &str) -> Result<Self, String> {
        struct Extract {
            templates: Vec<Template>,
            locations: Vec<(usize, usize)>,
            error: Option<String>,
        }
        impl VisitMut for Extract {
            fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
                if mac.path.segments.last().is_some_and(|s| s.ident == "view") {
                    match parse(mac.tokens.clone()) {
                        Ok(template) => {
                            self.templates.push(template);
                            let start = mac.path.segments.last().unwrap().ident.span().start();
                            self.locations.push((start.line, start.column + 1));
                            mac.tokens = TokenStream::new();
                        }
                        Err(error) => self.error = Some(error),
                    }
                } else {
                    syn::visit_mut::visit_macro_mut(self, mac);
                }
            }
        }
        let mut file = syn::parse_file(source).map_err(|e| e.to_string())?;
        let mut extract = Extract {
            templates: vec![],
            locations: vec![],
            error: None,
        };
        extract.visit_file_mut(&mut file);
        if let Some(error) = extract.error {
            return Err(error);
        }
        Ok(Self {
            outside: file.to_token_stream().to_string(),
            templates: extract.templates,
            locations: extract.locations,
        })
    }
    pub fn compatible(&self, next: &Self) -> Result<(), String> {
        if self.outside != next.outside || self.templates.len() != next.templates.len() {
            return Err("Rust outside view! changed".into());
        }
        for (old, new) in self.templates.iter().zip(&next.templates) {
            new.map_from(old)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(markup: &str) -> Source {
        Source::parse(&format!(
            "fn App() {{ let count = signal(0); view! {{ {markup} }} }}"
        ))
        .unwrap()
    }
    #[test]
    fn combined_logic_and_markup_is_never_a_partial_patch() {
        let before = source("<button onClick={move |_| count += 1}>\"Before\"</button>");
        let after = source("<button onClick={move |_| count += 2}>\"After\"</button>");
        assert!(before.compatible(&after).is_err());
        let after=Source::parse("fn App() { let count = signal(5); view! { <button onClick={move |_| count += 1}>\"After\"</button> } }").unwrap();
        assert!(before.compatible(&after).is_err());
    }
    #[test]
    fn static_changes_retain_compiled_component_and_binding_slots() {
        let before = source(
            "<view><Card title=\"stable\"/><button onClick={move |_| count += 1}>\"Add\"</button><span>\"Count: {count}\"</span></view>",
        );
        let after = source(
            "<div class=\"p-4\"><p>\"Inserted\"</p><span>\"Now: {count}\"</span><button onClick={move |_| count += 1}>\"Plus\"</button><Card title=\"stable\"/></div>",
        );
        before.compatible(&after).unwrap();
        let map = after.templates[0].map_from(&before.templates[0]).unwrap();
        for node in after.templates[0].flattened() {
            if let Kind::Hole(code) = &node.kind {
                assert_eq!(
                    &before.templates[0]
                        .flattened()
                        .into_iter()
                        .find(|old| old.id == map[&node.id])
                        .unwrap()
                        .kind,
                    &Kind::Hole(code.clone())
                );
            }
        }
    }
    #[test]
    fn malformed_edits_are_errors_before_any_template_exists() {
        for markup in [
            "<view><p></view>",
            "<view class=\"p-bad\"/>",
            "<view nonsense=\"x\"/>",
            "<view>\"{a.b}\"</view>",
        ] {
            assert!(
                Source::parse(&format!("fn App() {{ view! {{ {markup} }} }}")).is_err(),
                "{markup}"
            );
        }
    }
    #[test]
    fn literal_props_and_structural_moves_are_template_edits() {
        assert!(
            source("<view><Card title=\"old\"/></view>")
                .compatible(&source("<view><Card title=\"new\"/></view>"))
                .is_ok()
        );
        assert!(
            source("<view><div><Card/></div></view>")
                .compatible(&source("<view><Card/><div/></view>"))
                .is_ok()
        );
        assert!(
            source("<view><div>{move || Some(view!{<p/>})}</div></view>")
                .compatible(&source("<view>{move || Some(view!{<p/>})}<div/></view>"))
                .is_ok()
        );
        assert!(
            source("<view><Card title=\"one\"/><Card title=\"two\"/></view>")
                .compatible(&source(
                    "<view><Card title=\"two\"/><Card title=\"one\"/></view>"
                ))
                .is_err()
        );
        assert!(
            source("<view><div id=\"left\"><Card title=\"one\"/></div><div id=\"right\"><Card title=\"two\"/></div></view>")
                .compatible(&source("<view><div id=\"left\"/><div id=\"right\"><Card title=\"one\"/><Card title=\"two\"/></div></view>"))
                .is_err()
        );
        assert!(
            source("<view><div id=\"left\"><Card id=\"one\"/></div><div id=\"right\"><Card id=\"two\"/></div></view>")
                .compatible(&source("<view><div id=\"left\"/><div id=\"right\"><Card id=\"one\"/><Card id=\"two\"/></div></view>"))
                .is_ok()
        );
        assert!(
            source("<view>{move || Some(view!{<p/>})}</view>")
                .compatible(&source("<view><p/> {move || Some(view!{<p/>})}</view>"))
                .is_ok()
        );
    }
}
