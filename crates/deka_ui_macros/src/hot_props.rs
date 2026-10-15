//! Reloadable scalar reads retain ordinary Rust parameter types. Uses outside
//! direct reactive markup remain compiled and therefore force a restart.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit_mut::VisitMut;

fn scalar(ty: &Type) -> bool {
    static_str(ty)
        || matches!(ty, Type::Path(path) if path.qself.is_none() && path.path.segments.last().is_some_and(|s|
        matches!(s.ident.to_string().as_str(), "String" | "bool" | "char" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f32" | "f64")))
}
fn static_str(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference) if reference.mutability.is_none() && reference.lifetime.as_ref().is_some_and(|lifetime| lifetime.ident == "static") && matches!(reference.elem.as_ref(), Type::Path(path) if path.path.is_ident("str")))
}
fn names(tokens: Tokens, blocked: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Ident(name) => {
                blocked.insert(name.to_string());
            }
            proc_macro2::TokenTree::Group(group) => names(group.stream(), blocked),
            proc_macro2::TokenTree::Literal(value) => {
                if let Ok(value) = syn::parse_str::<LitStr>(&value.to_string()) {
                    blocked.extend(
                        value
                            .value()
                            .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                            .filter(|word| !word.is_empty())
                            .map(str::to_owned),
                    );
                }
            }
            _ => (),
        }
    }
}
fn inspect_live(expr: &Expr, blocked: &mut BTreeSet<String>) {
    struct Inspect<'a>(&'a mut BTreeSet<String>);
    impl VisitMut for Inspect<'_> {
        fn visit_pat_mut(&mut self, pattern: &mut Pat) {
            names(pattern.to_token_stream(), self.0);
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            match expr {
                Expr::Assign(assign) => names(assign.left.to_token_stream(), self.0),
                // A proc macro cannot prove a receiver method is read-only.
                // Preserve its ordinary ownership/mutation semantics.
                Expr::MethodCall(call) => names(call.receiver.to_token_stream(), self.0),
                Expr::Binary(binary)
                    if matches!(
                        binary.op,
                        syn::BinOp::AddAssign(_)
                            | syn::BinOp::SubAssign(_)
                            | syn::BinOp::MulAssign(_)
                            | syn::BinOp::DivAssign(_)
                            | syn::BinOp::RemAssign(_)
                            | syn::BinOp::BitXorAssign(_)
                            | syn::BinOp::BitAndAssign(_)
                            | syn::BinOp::BitOrAssign(_)
                            | syn::BinOp::ShlAssign(_)
                            | syn::BinOp::ShrAssign(_)
                    ) =>
                {
                    names(binary.left.to_token_stream(), self.0)
                }
                Expr::Reference(reference) if reference.mutability.is_some() => {
                    names(reference.expr.to_token_stream(), self.0)
                }
                _ => (),
            }
            syn::visit_mut::visit_expr_mut(self, expr);
        }
        fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
            names(mac.tokens.clone(), self.0);
        }
    }
    Inspect(blocked).visit_expr_mut(&mut expr.clone());
}
pub(super) fn live_expression(expr: &Expr, props: &BTreeMap<String, Ident>) -> Expr {
    struct Rewrite<'a>(&'a BTreeMap<String, Ident>);
    impl VisitMut for Rewrite<'_> {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if let Expr::Path(path) = expr
                && let Some(name) = path.path.get_ident()
                && let Some(signal) = self.0.get(&name.to_string())
            {
                *expr = syn::parse_quote!(#signal.get().unwrap());
            } else {
                syn::visit_mut::visit_expr_mut(self, expr);
            }
        }
    }
    let mut result = expr.clone();
    Rewrite(props).visit_expr_mut(&mut result);
    result
}
fn inspect(nodes: &[Node], blocked: &mut BTreeSet<String>) {
    for node in nodes {
        match node {
            Node::Element(element) => {
                let component = element
                    .name()
                    .to_string()
                    .rsplit("::")
                    .next()
                    .is_some_and(|s| s.trim().starts_with(char::is_uppercase));
                if component {
                    names(element.to_token_stream(), blocked);
                    continue;
                }
                for attr in element.attributes() {
                    if let NodeAttribute::Attribute(attr) = attr
                        && let Some(expr) = attr.value()
                    {
                        let key = attr.key.to_string();
                        if (deka_native_ir::is_supported_attribute(&key)
                            || key == "value"
                            || key.starts_with("class:"))
                            && matches!(unwrap_value(expr), Expr::Path(path) if path.path.get_ident().is_some())
                        {
                            continue;
                        }
                        if matches!(unwrap_value(expr), Expr::Closure(closure) if closure.capture.is_some())
                        {
                            inspect_live(unwrap_value(expr), blocked);
                            continue;
                        }
                        names(expr.to_token_stream(), blocked);
                    }
                }
                inspect(&element.children, blocked);
            }
            Node::Fragment(fragment) => inspect(&fragment.children, blocked),
            Node::Block(block) => {
                if let Some(block) = block.try_block()
                    && let [syn::Stmt::Expr(expr, None)] = block.stmts.as_slice()
                    && matches!(expr, Expr::Closure(closure) if closure.capture.is_some())
                {
                    inspect_live(expr, blocked);
                } else {
                    names(block.to_token_stream(), blocked);
                }
            }
            _ => (),
        }
    }
}
pub(super) fn component_body(body: &syn::Block, props: &[Prop]) -> syn::Result<Tokens> {
    struct Inspect {
        blocked: BTreeSet<String>,
    }
    impl VisitMut for Inspect {
        fn visit_expr_path_mut(&mut self, path: &mut syn::ExprPath) {
            names(path.to_token_stream(), &mut self.blocked);
        }
        fn visit_pat_mut(&mut self, pattern: &mut Pat) {
            // A shadowed identifier cannot silently refer to the prop signal.
            names(pattern.to_token_stream(), &mut self.blocked);
        }
        fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
            if mac.path.segments.last().is_some_and(|s| s.ident == "view") {
                if let Ok(nodes) = rstml::parse2(mac.tokens.clone()) {
                    inspect(&nodes, &mut self.blocked);
                }
            } else {
                names(mac.tokens.clone(), &mut self.blocked);
            }
        }
    }
    let mut inspected = body.clone();
    let mut inspector = Inspect {
        blocked: BTreeSet::new(),
    };
    inspector.visit_block_mut(&mut inspected);
    let signals: BTreeMap<_, _> = props
        .iter()
        .filter(|p| scalar(&p.ty) && !inspector.blocked.contains(&p.name.to_string()))
        .map(|p| {
            (
                p.name.to_string(),
                format_ident!("__deka_hot_prop_{}", p.name, span = Span::mixed_site()),
            )
        })
        .collect();
    struct Rewrite<'a> {
        signals: &'a BTreeMap<String, Ident>,
        error: Option<syn::Error>,
    }
    impl VisitMut for Rewrite<'_> {
        fn visit_stmt_mut(&mut self, stmt: &mut syn::Stmt) {
            if let syn::Stmt::Macro(mac) = stmt
                && mac
                    .mac
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "view")
            {
                match markup_with_props(
                    mac.mac.tokens.clone(),
                    Some(mac.mac.path.segments.last().unwrap().ident.span()),
                    self.signals,
                )
                .and_then(syn::parse2::<Expr>)
                {
                    Ok(mut replacement) => {
                        if let Expr::Block(block) = &mut replacement {
                            block.attrs.extend(mac.attrs.clone());
                        }
                        *stmt = syn::Stmt::Expr(replacement, mac.semi_token);
                    }
                    Err(error) => self.error = Some(error),
                }
            } else {
                syn::visit_mut::visit_stmt_mut(self, stmt);
            }
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if let Expr::Macro(mac) = expr
                && mac
                    .mac
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "view")
            {
                match markup_with_props(
                    mac.mac.tokens.clone(),
                    Some(mac.mac.path.segments.last().unwrap().ident.span()),
                    self.signals,
                )
                .and_then(syn::parse2)
                {
                    Ok(replacement) => *expr = replacement,
                    Err(error) => self.error = Some(error),
                }
            } else {
                syn::visit_mut::visit_expr_mut(self, expr);
            }
        }
    }
    let mut hot = body.clone();
    let mut rewrite = Rewrite {
        signals: &signals,
        error: None,
    };
    rewrite.visit_block_mut(&mut hot);
    if let Some(error) = rewrite.error {
        return Err(error);
    }
    let bindings = props.iter().filter_map(|p| {
        let signal = signals.get(&p.name.to_string())?;
        let name = &p.name;
        let key = name.to_string();
        let value = if static_str(&p.ty) {
            quote!(#name.to_owned())
        } else {
            quote!(#name)
        };
        Some(quote!(let #signal = ::deka_ui::hot_reload::prop(#key, #value);))
    });
    let statements = &hot.stmts;
    Ok(quote!(#(#bindings)* #(#statements)*))
}
