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
                    output.push(TemplateNode {
                        id,
                        kind: Kind::Hole(element.to_token_stream().to_string()),
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
        let mut result = BTreeMap::new();
        let mut used = std::collections::BTreeSet::new();
        for new in &new_nodes {
            if let Some(key) = compiled(new) {
                let previous = old_nodes
                    .iter()
                    .find(|n| !used.contains(&n.id) && compiled(n).as_ref() == Some(&key))
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
        fn parents(
            nodes: &[TemplateNode],
            parent: Option<usize>,
            out: &mut BTreeMap<usize, Option<usize>>,
        ) {
            for node in nodes {
                out.insert(node.id, parent);
                parents(&node.children, Some(node.id), out);
            }
        }
        let mut old_parents = BTreeMap::new();
        parents(&old.nodes, None, &mut old_parents);
        let mut new_parents = BTreeMap::new();
        parents(&self.nodes, None, &mut new_parents);
        for node in self.flattened() {
            if matches!(&node.kind,Kind::Hole(code) if !code.starts_with("interpolation:")) {
                let previous = result[&node.id];
                if new_parents[&node.id].and_then(|p| result.get(&p).copied())
                    != old_parents[&previous]
                {
                    return Err(
                        "moving a compiled child to another parent requires rebuilding".into(),
                    );
                }
            }
            if matches!(&node.kind,Kind::Hole(code) if code.starts_with('{')) {
                let previous = result[&node.id];
                if new_parents[&node.id].and_then(|p| result.get(&p).copied())
                    != old_parents[&previous]
                {
                    return Err(
                        "moving a Rust child slot to another parent requires rebuilding".into(),
                    );
                }
                // Reactive Rust children keep their original structural slot
                // paths. Until those paths can move, changing their sibling
                // topology would make the next signal turn reorder the tree.
                fn siblings(template: &Template, parent: Option<usize>) -> &[TemplateNode] {
                    parent.map_or(template.nodes.as_slice(), |id| {
                        template
                            .flattened()
                            .into_iter()
                            .find(|n| n.id == id)
                            .unwrap()
                            .children
                            .as_slice()
                    })
                }
                let before: Vec<_> = siblings(old, old_parents[&previous])
                    .iter()
                    .map(|n| Some(n.id))
                    .collect();
                let after: Vec<_> = siblings(self, new_parents[&node.id])
                    .iter()
                    .map(|n| result.get(&n.id).copied())
                    .collect();
                if before != after {
                    return Err(
                        "changing siblings around a Rust child slot requires rebuilding".into(),
                    );
                }
            }
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
    fn component_literal_props_and_structural_slot_moves_rebuild() {
        assert!(
            source("<view><Card title=\"old\"/></view>")
                .compatible(&source("<view><Card title=\"new\"/></view>"))
                .is_err()
        );
        assert!(
            source("<view><div><Card/></div></view>")
                .compatible(&source("<view><Card/><div/></view>"))
                .is_err()
        );
        assert!(
            source("<view><div>{move || Some(view!{<p/>})}</div></view>")
                .compatible(&source("<view>{move || Some(view!{<p/>})}<div/></view>"))
                .is_err()
        );
        assert!(
            source("<view>{move || Some(view!{<p/>})}</view>")
                .compatible(&source("<view><p/> {move || Some(view!{<p/>})}</view>"))
                .is_err()
        );
    }
}
