//! Rust token/span preserving markup and ordinary function components.
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span, TokenStream as Tokens};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use rstml::node::{Node, NodeAttribute, NodeName};
use syn::{Expr, FnArg, ItemFn, Lit, LitStr, Pat, Type, spanned::Spanned};

#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    markup(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
fn markup(input: Tokens) -> syn::Result<Tokens> {
    nodes(&rstml::parse2(input)?)
}
fn nodes(input: &[Node]) -> syn::Result<Tokens> {
    let values = input
        .iter()
        .filter(|node| !matches!(node, Node::Comment(_)))
        .map(node)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(if values.len() == 1 {
        values[0].clone()
    } else {
        quote!(::deka_ui::View::fragment(vec![#(#values),*]))
    })
}
fn node(input: &Node) -> syn::Result<Tokens> {
    match input {
        Node::Text(text) => text_nodes(&text.value),
        Node::Block(block) => {
            let block = block
                .try_block()
                .ok_or_else(|| syn::Error::new_spanned(block, "invalid Rust child"))?;
            let value = if let [syn::Stmt::Expr(expr, None)] = block.stmts.as_slice() {
                expr.to_token_stream()
            } else {
                block.to_token_stream()
            };
            Ok(quote!(::deka_ui::View::from_child(#value)))
        }
        Node::Fragment(fragment) => nodes(&fragment.children),
        Node::Element(element) => {
            let name = &element.open_tag.name;
            let tag = name.to_string();
            let component = match name {
                NodeName::Path(path) => path
                    .path
                    .segments
                    .last()
                    .is_some_and(|p| p.ident.to_string().starts_with(char::is_uppercase)),
                _ => false,
            };
            if !component {
                deka_native_ir::WireNode {
                    tag: tag.clone(),
                    ..Default::default()
                }
                .style()
                .map_err(|error| syn::Error::new_spanned(name, error))?;
            }
            let mut seen = std::collections::BTreeSet::new();
            let mut setters = Vec::new();
            for attribute in element.attributes() {
                let NodeAttribute::Attribute(attribute) = attribute else {
                    return Err(syn::Error::new_spanned(attribute, "use a named attribute"));
                };
                let key = attribute.key.to_string();
                if !seen.insert(key.clone()) {
                    return Err(syn::Error::new_spanned(
                        &attribute.key,
                        "duplicate attribute",
                    ));
                }
                let value = attribute.value().ok_or_else(|| {
                    syn::Error::new_spanned(attribute, "attribute requires a Rust value")
                })?;
                let value = unwrap_value(value);
                let span = attribute.key.span();
                if component {
                    let mut method: Ident = syn::parse_str(&key).map_err(|_| {
                        syn::Error::new_spanned(
                            &attribute.key,
                            "component props must be Rust identifiers",
                        )
                    })?;
                    method.set_span(span);
                    let value = match value {
                        Expr::Lit(lit) if matches!(lit.lit, Lit::Str(_)) => {
                            quote_spanned!(value.span()=> (#value).into())
                        }
                        _ => value.to_token_stream(),
                    };
                    setters.push(quote_spanned!(span=> .#method(#value)));
                } else if key == "value" {
                    setters.push(quote_spanned!(span=> .value(#value)));
                } else if key == "node_ref" {
                    setters.push(quote_spanned!(span=> .node_ref(#value)));
                } else if let Some(class) = key.strip_prefix("class:") {
                    if class.is_empty() || class.chars().any(char::is_whitespace) {
                        return Err(syn::Error::new_spanned(
                            &attribute.key,
                            "class toggle requires one utility",
                        ));
                    }
                    deka_native_ir::apply_classes(&mut Default::default(), class)
                        .map_err(|error| syn::Error::new_spanned(&attribute.key, error))?;
                    setters.push(quote_spanned!(span=> .class(#class, #value)));
                } else if let Some(event) = match key.as_str() {
                    "onClick" => Some(quote!(Click)),
                    "onInput" => Some(quote!(Input)),
                    "onKeyDown" => Some(quote!(KeyDown)),
                    "onContextMenu" => Some(quote!(ContextMenu)),
                    _ => None,
                } {
                    setters.push(quote_spanned!(span=> .on(::deka_ui::EventKind::#event, #value)));
                } else if deka_native_ir::is_supported_attribute(&key) {
                    if key == "className"
                        && let Some(classes) = attribute.value_literal_string()
                    {
                        deka_native_ir::apply_classes(&mut Default::default(), &classes)
                            .map_err(|error| syn::Error::new_spanned(value, error))?;
                    }
                    setters.push(quote_spanned!(span=> .attr(#key, #value)));
                } else {
                    return Err(syn::Error::new_spanned(
                        &attribute.key,
                        if key == "ref" {
                            "node refs belong to phase 2"
                        } else {
                            "unsupported native attribute"
                        },
                    ));
                }
            }
            let children = element
                .children
                .iter()
                .filter(|n| !matches!(n, Node::Comment(_)))
                .map(node)
                .collect::<syn::Result<Vec<_>>>()?;
            if component {
                let children = if children.is_empty() {
                    Tokens::new()
                } else {
                    if seen.contains("children") {
                        return Err(syn::Error::new_spanned(
                            name,
                            "children were supplied twice",
                        ));
                    }
                    quote_spanned!(name.span()=> .children(Box::new(move || ::deka_ui::View::fragment(vec![#(#children),*]))))
                };
                Ok(
                    quote_spanned!(name.span()=> #name(::deka_ui::props_for(#name) #(#setters)* #children .build())),
                )
            } else {
                Ok(
                    quote_spanned!(name.span()=> ::deka_ui::View::element(#tag) #(#setters)* #(.child(#children))*),
                )
            }
        }
        Node::Comment(_) => unreachable!("comments are filtered before expansion"),
        _ => Err(syn::Error::new_spanned(
            input,
            "text must be quoted; Rust children go inside braces",
        )),
    }
}
fn unwrap_value(mut value: &Expr) -> &Expr {
    while let Expr::Block(block) = value {
        if !block.attrs.is_empty() || block.label.is_some() {
            break;
        }
        if let [syn::Stmt::Expr(expr, None)] = block.block.stmts.as_slice() {
            value = expr;
        } else {
            break;
        }
    }
    value
}
fn text_nodes(literal: &LitStr) -> syn::Result<Tokens> {
    let source = literal.value();
    let mut text = String::new();
    let mut parts = Vec::new();
    let mut chars = source.chars().enumerate().peekable();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '{' if chars.peek().map(|(_, ch)| ch) == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek().map(|(_, ch)| ch) == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                if !text.is_empty() {
                    let value = LitStr::new(&std::mem::take(&mut text), literal.span());
                    parts.push(quote_spanned!(literal.span()=> ::deka_ui::View::text(#value)));
                }
                let mut name = String::new();
                let mut closed = false;
                for (_, ch) in chars.by_ref() {
                    if ch == '}' {
                        closed = true;
                        break;
                    }
                    name.push(ch);
                }
                if !closed {
                    return Err(interpolation_error(
                        literal,
                        index,
                        "unclosed text interpolation",
                    ));
                }
                let mut name: Ident = syn::parse_str(&name).map_err(|_| {
                    interpolation_error(literal, index, "text interpolation requires an identifier")
                })?;
                name.set_span(literal.span());
                parts.push(quote_spanned!(literal.span()=> ::deka_ui::View::interpolate(&#name)));
            }
            '}' => {
                return Err(interpolation_error(
                    literal,
                    index,
                    "unmatched text interpolation brace",
                ));
            }
            _ => text.push(ch),
        }
    }
    if !text.is_empty() || parts.is_empty() {
        let value = LitStr::new(&text, literal.span());
        parts.push(quote_spanned!(literal.span()=> ::deka_ui::View::text(#value)));
    }
    Ok(if parts.len() == 1 {
        parts.pop().unwrap()
    } else {
        quote!(::deka_ui::View::fragment(vec![#(#parts),*]))
    })
}

/// Map decoded characters back to their original literal bytes, including raw
/// strings, escaped Unicode and escaped-newline whitespace continuation.
fn literal_offsets(source: &str) -> Vec<usize> {
    let start = source.find('"').unwrap_or(0) + 1;
    let end = source.rfind('"').unwrap_or(source.len());
    let raw = source.starts_with('r');
    let mut offsets = Vec::new();
    let mut index = start;
    while index < end {
        let offset = index;
        let ch = source[index..].chars().next().unwrap();
        index += ch.len_utf8();
        // Rust normalizes a physical CRLF to one decoded newline in both
        // ordinary and raw string literals; keep one source-offset entry.
        if ch == '\r' && source.as_bytes().get(index) == Some(&b'\n') {
            index += 1;
        }
        if !raw && ch == '\\' && index < end {
            let escaped = source[index..].chars().next().unwrap();
            index += escaped.len_utf8();
            match escaped {
                'u' => {
                    if let Some(close) = source[index..end].find('}') {
                        index += close + 1;
                    }
                }
                'x' => index = (index + 2).min(end),
                '\n' | '\r' => {
                    while index < end {
                        let ch = source[index..].chars().next().unwrap();
                        if !ch.is_whitespace() {
                            break;
                        }
                        index += ch.len_utf8();
                    }
                    continue;
                }
                _ => {}
            }
        }
        offsets.push(offset);
    }
    offsets
}
fn interpolation_error(literal: &LitStr, index: usize, message: &str) -> syn::Error {
    let token = literal.token();
    let source = literal
        .span()
        .source_text()
        .unwrap_or_else(|| token.to_string());
    let offset = literal_offsets(&source).get(index).copied().unwrap_or(0);
    let width = source[offset..].chars().next().map_or(0, char::len_utf8);
    let span = token
        .subspan(offset..offset + width)
        .unwrap_or(literal.span());
    let start = literal.span().start();
    let (mut line, mut column) = (start.line, start.column + 1);
    for ch in source[..offset].chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    syn::Error::new(
        span,
        format!("{message} (brace at line {line}, column {column})"),
    )
}

#[proc_macro_attribute]
pub fn component(attributes: TokenStream, item: TokenStream) -> TokenStream {
    if !attributes.is_empty() {
        return syn::Error::new(Span::call_site(), "component takes no arguments")
            .into_compile_error()
            .into();
    }
    syn::parse::<ItemFn>(item)
        .and_then(expand_component)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
struct Prop {
    name: Ident,
    pattern: Pat,
    ty: Type,
    default: Option<Tokens>,
}
fn expand_component(mut function: ItemFn) -> syn::Result<Tokens> {
    if !function.sig.generics.params.is_empty()
        || function.sig.asyncness.is_some()
        || matches!(function.sig.safety, syn::Safety::Unsafe(_))
        || function.sig.abi.is_some()
        || function.sig.constness.is_some()
        || function.sig.variadic.is_some()
    {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "components require ordinary non-generic Rust functions",
        ));
    }
    if !matches!(&function.sig.output,syn::ReturnType::Type(_,ty) if matches!(ty.as_ref(),Type::Path(path) if path.path.segments.last().is_some_and(|p|p.ident=="View")))
    {
        return Err(syn::Error::new_spanned(
            &function.sig.output,
            "component must return View",
        ));
    }
    let mut props = Vec::new();
    for input in &function.sig.inputs {
        let FnArg::Typed(input) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "component props cannot be receivers",
            ));
        };
        let Pat::Ident(pattern) = input.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &input.pat,
                "component props must be named",
            ));
        };
        if pattern.by_ref.is_some() || pattern.subpat.is_some() {
            return Err(syn::Error::new_spanned(
                pattern,
                "component props must be owned identifiers",
            ));
        }
        let mut default = None;
        for attribute in &input.attrs {
            if !attribute.path().is_ident("prop") {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "unsupported prop attribute",
                ));
            }
            attribute.parse_nested_meta(|meta| {
                if !meta.path.is_ident("default") || default.is_some() {
                    return Err(meta.error("expected one prop default"));
                }
                default = Some(if meta.input.peek(syn::Token![=]) {
                    let value: Expr = meta.value()?.parse()?;
                    quote!(#value)
                } else {
                    quote!(::core::default::Default::default())
                });
                Ok(())
            })?;
        }
        if default.is_none()
            && matches!(input.ty.as_ref(),Type::Path(path) if path.path.segments.last().is_some_and(|p|p.ident=="Children"))
        {
            default = Some(quote!(Box::new(|| ::deka_ui::View::fragment(Vec::<
                ::deka_ui::View,
            >::new(
            )))));
        }
        props.push(Prop {
            name: pattern.ident.clone(),
            pattern: (*input.pat).clone(),
            ty: (*input.ty).clone(),
            default,
        });
    }
    let name = &function.sig.ident;
    let vis = &function.vis;
    let props_name = format_ident!("{}Props", name);
    let builder = format_ident!("{}PropsBuilder", name);
    let missing = format_ident!("{}MissingProp", name);
    let required: Vec<_> = props.iter().filter(|p| p.default.is_none()).collect();
    let params: Vec<_> = (0..required.len()).map(|i| format_ident!("P{i}")).collect();
    let builder_type = |args: &[Tokens]| {
        if args.is_empty() {
            quote!(#builder)
        } else {
            quote!(#builder<#(#args),*>)
        }
    };
    let initial_type = builder_type(
        &required
            .iter()
            .map(|_| quote!(#missing))
            .collect::<Vec<_>>(),
    );
    let requirements: Vec<_> = required
        .iter()
        .enumerate()
        .map(|(index, _)| format_ident!("{}RequiredProp{}", name, index))
        .collect();
    let requirement_definitions = required.iter().zip(&requirements).map(|(p, requirement)| {
        let ty = &p.ty;
        let message = format!("missing required prop `{}` for component `{name}`", p.name);
        quote!(
            #[doc(hidden)] #[diagnostic::on_unimplemented(message = #message)]
            #vis trait #requirement { fn into_prop(self) -> #ty; }
            impl #requirement for #ty { fn into_prop(self) -> #ty { self } }
        )
    });
    let build_bounds = if required.is_empty() {
        Tokens::new()
    } else {
        quote!(where #(#params: #requirements),*)
    };
    let missing_definition = if required.is_empty() {
        Tokens::new()
    } else {
        quote!(#[doc(hidden)] #vis struct #missing;)
    };
    let generic_args: Vec<_> = params.iter().map(|p| quote!(#p)).collect();
    let implementation = if params.is_empty() {
        Tokens::new()
    } else {
        quote!(<#(#params),*>)
    };
    let definition = if params.is_empty() {
        Tokens::new()
    } else {
        quote!(<#(#params=#missing),*>)
    };
    let mut builder_fields = Vec::new();
    let mut initial_fields = Vec::new();
    let mut complete_fields = Vec::new();
    let mut methods = Vec::new();
    for prop in &props {
        let field = &prop.name;
        let ty = &prop.ty;
        let index = required.iter().position(|p| p.name == *field);
        if let Some(index) = index {
            let param = &params[index];
            builder_fields.push(quote!(#field:#param));
            initial_fields.push(quote!(#field:#missing));
            let requirement = &requirements[index];
            complete_fields.push(quote!(#field:#requirement::into_prop(self.#field)));
        } else {
            let default = prop.default.as_ref().unwrap();
            builder_fields.push(quote!(#field:Option<#ty>));
            initial_fields.push(quote!(#field:None));
            complete_fields.push(quote!(#field:self.#field.unwrap_or_else(|| #default)));
        }
        let mut next = generic_args.clone();
        if let Some(index) = index {
            next[index] = quote!(#ty);
        }
        let returned = builder_type(&next);
        let fields = props.iter().map(|other| {
            let name = &other.name;
            if name == field {
                if index.is_some() {
                    quote!(#name:value)
                } else {
                    quote!(#name:Some(value))
                }
            } else {
                quote!(#name:self.#name)
            }
        });
        methods.push(quote_spanned!(field.span()=> #[allow(dead_code, reason = "generated setters support optional markup and direct props construction")] pub fn #field(self,value:#ty)->#returned {#builder{#(#fields),*}}));
    }
    let names = props.iter().map(|p| &p.name);
    let types = props.iter().map(|p| &p.ty);
    let patterns = props.iter().map(|p| &p.pattern);
    let default = if required.is_empty() {
        quote!(impl ::core::default::Default for #props_name {fn default()->Self {Self::builder().build()}})
    } else {
        Tokens::new()
    };
    let body = &function.block;
    let props_binding = Ident::new("__deka_props", Span::mixed_site());
    let replacement: syn::Block =
        syn::parse2(quote!({let #props_name{#(#patterns),*}=#props_binding; #body}))?;
    function.sig.inputs = syn::parse_quote!(#props_binding:#props_name);
    *function.block = replacement;
    // Component names intentionally match capitalized Rust markup tags.
    function
        .attrs
        .push(syn::parse_quote!(#[allow(non_snake_case)]));
    Ok(quote!(
        #vis struct #props_name {#(pub #names:#types),*}
        #missing_definition
        #(#requirement_definitions)*
        #[doc(hidden)] #vis struct #builder #definition {#(#builder_fields),*}
        impl #props_name {pub fn builder()->#initial_type {#builder{#(#initial_fields),*}}}
        impl ::deka_ui::ComponentProps for #props_name {type Builder=#initial_type;fn builder()->Self::Builder {Self::builder()}}
        impl #implementation #builder #implementation {#(#methods)*}
        impl #implementation #builder #implementation {#[allow(dead_code, reason = "generated builder supports both markup and direct props construction")] pub fn build(self)->#props_name #build_bounds {#props_name{#(#complete_fields),*}}}
        #default
        #function
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_crlf_diagnostics_point_at_the_decoded_brace() {
        for source in ["\"First\r\nprefix {a.b}\"", "r#\"First\r\nprefix {a.b}\"#"] {
            let literal: LitStr = syn::parse_str(source).unwrap();
            // Index 13 is the brace after the compiler decodes physical CRLF
            // as one newline. The unit-test fallback parser retains raw CRLF.
            let error =
                interpolation_error(&literal, 13, "text interpolation requires an identifier");
            assert!(
                error.to_string().contains("brace at line 2, column 8"),
                "{error}"
            );
            assert_eq!(error.span().start().line, 2);
            assert_eq!(error.span().start().column, 7);
        }
    }
    #[test]
    fn interpolation_errors_locate_the_brace_through_raw_strings_and_escapes() {
        for source in [
            r#""prefix {a.b}""#,
            r##"r#"prefix {a.b}"#"##,
            r#""é\n{a.b}""#,
            r#""\u{00e9}{a.b}""#,
        ] {
            let literal: LitStr = syn::parse_str(source).unwrap();
            let error = text_nodes(&literal).unwrap_err();
            let offset = source.find("{a.b}").unwrap();
            assert_eq!(error.span().start().column, offset, "{source}");
            let column = source[..offset].chars().count() + 1;
            assert!(
                error
                    .to_string()
                    .contains(&format!("brace at line 1, column {column}")),
                "{error}"
            );
        }
        for source in [r#""prefix {a""#, r#""prefix }""#] {
            let literal: LitStr = syn::parse_str(source).unwrap();
            let error = text_nodes(&literal).unwrap_err();
            assert_eq!(error.span().start().column, 8);
        }
    }
    #[test]
    fn rstml_parses_rust_closures_colon_attributes_and_original_token_spans() {
        let source = "\n<view>\n<button onClick={move |_| count += 1} class:rounded={open}>\"Count: {count}\"</button>\n</view>";
        let tokens: Tokens = source.parse().unwrap();
        let parsed = rstml::parse2(tokens.clone()).unwrap();
        let Node::Element(root) = &parsed[0] else {
            panic!("element")
        };
        let Node::Element(button) = &root.children[0] else {
            panic!("button")
        };
        assert_eq!(button.open_tag.name.span().start().line, 3);
        let NodeAttribute::Attribute(event) = &button.attributes()[0] else {
            panic!("attribute")
        };
        assert!(matches!(
            unwrap_value(event.value().unwrap()),
            Expr::Closure(_)
        ));
        assert!(markup(tokens.clone()).is_ok());
        let arena = bumpalo::Bump::new();
        let stringified = tokens.to_string();
        assert!(!stringified.contains('\n'));
        let ds = format!("export fn App(){{return ({stringified});}}");
        let result = deka_syntax::parse::parse(&ds, &arena);
        assert!(result.errors.is_empty(), "{:#?}", result.errors);
        assert_eq!(
            result.program.unwrap().span.end.line,
            1,
            "stringification loses the original markup lines"
        );
        let ds = "export fn App(){return (<view><button onClick={fn(){}}>Count</button></view>);}";
        assert!(deka_syntax::parse::parse(ds, &arena).errors.is_empty());
    }
}
