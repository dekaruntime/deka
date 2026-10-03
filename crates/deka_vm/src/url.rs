//! Rust-owned URL state. URLSearchParams aliases stay linked to their URL.
use crate::Result;
use std::{cell::RefCell, rc::Rc};
use url::{Url, form_urlencoded, quirks};

#[derive(Clone, Debug)]
pub struct UrlObject {
    url: Rc<RefCell<Url>>,
    params: Rc<SearchParams>,
}

impl UrlObject {
    pub fn parse(input: &str, base: Option<&str>) -> Result<Self> {
        let base = base
            .map(Url::parse)
            .transpose()
            .map_err(|e| e.to_string())?;
        Url::options()
            .base_url(base.as_ref())
            .parse(input)
            .map(|url| {
                let url = Rc::new(RefCell::new(url));
                let params = Rc::new(SearchParams(Backing::Url(url.clone())));
                Self { url, params }
            })
            .map_err(|e| e.to_string())
    }
    pub fn href(&self) -> String {
        quirks::href(&self.url.borrow()).into()
    }
    pub fn origin(&self) -> String {
        quirks::origin(&self.url.borrow())
    }
    pub fn protocol(&self) -> String {
        quirks::protocol(&self.url.borrow()).into()
    }
    pub fn username(&self) -> String {
        quirks::username(&self.url.borrow()).into()
    }
    pub fn password(&self) -> String {
        quirks::password(&self.url.borrow()).into()
    }
    pub fn host(&self) -> String {
        quirks::host(&self.url.borrow()).into()
    }
    pub fn hostname(&self) -> String {
        quirks::hostname(&self.url.borrow()).into()
    }
    pub fn port(&self) -> String {
        quirks::port(&self.url.borrow()).into()
    }
    pub fn pathname(&self) -> String {
        quirks::pathname(&self.url.borrow()).into()
    }
    pub fn search(&self) -> String {
        quirks::search(&self.url.borrow()).into()
    }
    pub fn hash(&self) -> String {
        quirks::hash(&self.url.borrow()).into()
    }
    pub fn search_params(&self) -> Rc<SearchParams> {
        self.params.clone()
    }
}

#[derive(Clone, Debug)]
enum Backing {
    Url(Rc<RefCell<Url>>),
    List(Rc<RefCell<Vec<(String, String)>>>),
}
#[derive(Clone, Debug)]
pub struct SearchParams(Backing);

impl SearchParams {
    pub fn parse(input: &str) -> Self {
        let input = input.strip_prefix('?').unwrap_or(input);
        Self::from_pairs(
            form_urlencoded::parse(input.as_bytes())
                .into_owned()
                .collect(),
        )
    }
    pub fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        Self(Backing::List(Rc::new(RefCell::new(pairs))))
    }
    pub fn entries(&self) -> Vec<(String, String)> {
        match &self.0 {
            Backing::Url(url) => url.borrow().query_pairs().into_owned().collect(),
            Backing::List(list) => list.borrow().clone(),
        }
    }
    fn mutate(&self, edit: impl FnOnce(&mut Vec<(String, String)>)) {
        let mut pairs = self.entries();
        edit(&mut pairs);
        match &self.0 {
            Backing::Url(url) => {
                let query = encode(&pairs);
                url.borrow_mut()
                    .set_query(if query.is_empty() { None } else { Some(&query) });
            }
            Backing::List(list) => *list.borrow_mut() = pairs,
        }
    }
    pub fn size(&self) -> usize {
        self.entries().len()
    }
    pub fn append(&self, name: &str, value: &str) {
        self.mutate(|pairs| pairs.push((name.into(), value.into())));
    }
    pub fn delete(&self, name: &str, value: Option<&str>) {
        self.mutate(|pairs| {
            pairs.retain(|(key, item)| key != name || value.is_some_and(|v| v != item))
        });
    }
    pub fn get(&self, name: &str) -> Option<String> {
        self.entries()
            .into_iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }
    pub fn get_all(&self, name: &str) -> Vec<String> {
        self.entries()
            .into_iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value)
            .collect()
    }
    pub fn has(&self, name: &str, value: Option<&str>) -> bool {
        self.entries()
            .iter()
            .any(|(key, item)| key == name && value.is_none_or(|v| v == item))
    }
    pub fn set(&self, name: &str, value: &str) {
        self.mutate(|pairs| {
            let mut replaced = false;
            pairs.retain_mut(|(key, item)| {
                if key != name {
                    return true;
                }
                if replaced {
                    return false;
                }
                *item = value.into();
                replaced = true;
                true
            });
            if !replaced {
                pairs.push((name.into(), value.into()));
            }
        });
    }
    pub fn sort(&self) {
        // The web algorithm sorts by UTF-16 code units, preserving equal-name order.
        self.mutate(|pairs| pairs.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16())));
    }
    pub fn keys(&self) -> Vec<String> {
        self.entries().into_iter().map(|(key, _)| key).collect()
    }
    pub fn values(&self) -> Vec<String> {
        self.entries().into_iter().map(|(_, value)| value).collect()
    }
    pub fn serialize(&self) -> String {
        encode(&self.entries())
    }
}
fn encode(pairs: &[(String, String)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// All language signatures and dispatch handlers share the host registry.
pub fn register(hosts: &mut crate::Hosts) -> Result<()> {
    use crate::{HostHandle, HostOp, HostReply, HostType, HostValue};
    let url_ty = HostType::Handle("URL".into());
    let params_ty = HostType::Handle("URLSearchParams".into());
    let option_string = HostType::Option(Box::new(HostType::String));
    hosts.register(
        HostOp::new(
            "URL",
            vec![HostType::String, option_string.clone()],
            url_ty.clone(),
            false,
            None,
            |args| {
                let input = text(&args[0]);
                let base = optional_text(&args[1]);
                HostReply::Ready(
                    UrlObject::parse(input, base)
                        .map(|url| HostValue::Handle(HostHandle::new("URL", url))),
                )
            },
        )
        .with_result_channel()
        .with_global_binding()
        .with_defaults(vec![HostValue::Option(None)]),
    )?;
    hosts.register(
        HostOp::new(
            "URLSearchParams",
            vec![HostType::String],
            params_ty.clone(),
            false,
            None,
            |args| {
                HostReply::Ready(Ok(HostValue::Handle(HostHandle::new(
                    "URLSearchParams",
                    SearchParams::parse(text(&args[0])),
                ))))
            },
        )
        .with_global_binding()
        .with_defaults(vec![HostValue::String(String::new())]),
    )?;
    type UrlGetter = fn(&UrlObject) -> String;
    let getters: &[(&str, UrlGetter)] = &[
        ("href", UrlObject::href),
        ("origin", UrlObject::origin),
        ("protocol", UrlObject::protocol),
        ("username", UrlObject::username),
        ("password", UrlObject::password),
        ("host", UrlObject::host),
        ("hostname", UrlObject::hostname),
        ("port", UrlObject::port),
        ("pathname", UrlObject::pathname),
        ("search", UrlObject::search),
        ("hash", UrlObject::hash),
    ];
    for &(name, getter) in getters {
        hosts.register(
            HostOp::new(
                &format!("url_{name}"),
                vec![url_ty.clone()],
                HostType::String,
                false,
                None,
                move |args| {
                    HostReply::Ready(
                        url_resource(&args[0]).map(|url| HostValue::String(getter(url))),
                    )
                },
            )
            .with_receiver_property("URL", name),
        )?;
    }
    hosts.register(
        HostOp::new(
            "url_searchParams",
            vec![url_ty.clone()],
            params_ty.clone(),
            false,
            None,
            |args| {
                HostReply::Ready(url_resource(&args[0]).map(|url| {
                    HostValue::Handle(HostHandle::from_shared(
                        "URLSearchParams",
                        url.search_params(),
                    ))
                }))
            },
        )
        .with_receiver_property("URL", "searchParams"),
    )?;
    for name in ["toString", "toJSON"] {
        hosts.register(
            HostOp::new(
                &format!("url_{name}"),
                vec![url_ty.clone()],
                HostType::String,
                false,
                None,
                |args| {
                    HostReply::Ready(
                        url_resource(&args[0]).map(|url| HostValue::String(url.href())),
                    )
                },
            )
            .with_receiver_method("URL", name),
        )?;
    }
    hosts.register(
        HostOp::new(
            "params_size",
            vec![params_ty.clone()],
            HostType::Number,
            false,
            None,
            |args| {
                HostReply::Ready(
                    params_resource(&args[0]).map(|params| HostValue::Number(params.size() as f64)),
                )
            },
        )
        .with_receiver_property("URLSearchParams", "size"),
    )?;
    hosts.register(
        HostOp::new(
            "params_get",
            vec![params_ty.clone(), HostType::String],
            option_string.clone(),
            false,
            None,
            |args| {
                HostReply::Ready(params_resource(&args[0]).map(|params| {
                    HostValue::Option(
                        params
                            .get(text(&args[1]))
                            .map(|value| Box::new(HostValue::String(value))),
                    )
                }))
            },
        )
        .with_receiver_method("URLSearchParams", "get"),
    )?;
    hosts.register(
        HostOp::new(
            "params_getAll",
            vec![params_ty.clone(), HostType::String],
            HostType::Strings,
            false,
            None,
            |args| {
                HostReply::Ready(
                    params_resource(&args[0])
                        .map(|params| HostValue::Strings(params.get_all(text(&args[1])))),
                )
            },
        )
        .with_receiver_method("URLSearchParams", "getAll"),
    )?;
    for (name, action) in [
        (
            "append",
            SearchParams::append as fn(&SearchParams, &str, &str),
        ),
        ("set", SearchParams::set),
    ] {
        hosts.register(
            HostOp::new(
                &format!("params_{name}"),
                vec![params_ty.clone(), HostType::String, HostType::String],
                HostType::Unit,
                false,
                None,
                move |args| {
                    HostReply::Ready(params_resource(&args[0]).map(|params| {
                        action(params, text(&args[1]), text(&args[2]));
                        HostValue::Unit
                    }))
                },
            )
            .with_receiver_method("URLSearchParams", name),
        )?;
    }
    hosts.register(
        HostOp::new(
            "params_delete",
            vec![params_ty.clone(), HostType::String, option_string.clone()],
            HostType::Unit,
            false,
            None,
            |args| {
                HostReply::Ready(params_resource(&args[0]).map(|params| {
                    params.delete(text(&args[1]), optional_text(&args[2]));
                    HostValue::Unit
                }))
            },
        )
        .with_receiver_method("URLSearchParams", "delete")
        .with_defaults(vec![HostValue::Option(None)]),
    )?;
    hosts.register(
        HostOp::new(
            "params_has",
            vec![params_ty.clone(), HostType::String, option_string],
            HostType::Bool,
            false,
            None,
            |args| {
                HostReply::Ready(params_resource(&args[0]).map(|params| {
                    HostValue::Bool(params.has(text(&args[1]), optional_text(&args[2])))
                }))
            },
        )
        .with_receiver_method("URLSearchParams", "has")
        .with_defaults(vec![HostValue::Option(None)]),
    )?;
    hosts.register(
        HostOp::new(
            "params_sort",
            vec![params_ty.clone()],
            HostType::Unit,
            false,
            None,
            |args| {
                HostReply::Ready(params_resource(&args[0]).map(|params| {
                    params.sort();
                    HostValue::Unit
                }))
            },
        )
        .with_receiver_method("URLSearchParams", "sort"),
    )?;
    hosts.register(
        HostOp::new(
            "params_toString",
            vec![params_ty.clone()],
            HostType::String,
            false,
            None,
            |args| {
                HostReply::Ready(
                    params_resource(&args[0]).map(|params| HostValue::String(params.serialize())),
                )
            },
        )
        .with_receiver_method("URLSearchParams", "toString"),
    )?;
    for (name, action) in [
        (
            "keys",
            SearchParams::keys as fn(&SearchParams) -> Vec<String>,
        ),
        ("values", SearchParams::values),
    ] {
        hosts.register(
            HostOp::new(
                &format!("params_{name}"),
                vec![params_ty.clone()],
                HostType::Strings,
                false,
                None,
                move |args| {
                    HostReply::Ready(
                        params_resource(&args[0]).map(|params| HostValue::Strings(action(params))),
                    )
                },
            )
            .with_receiver_method("URLSearchParams", name),
        )?;
    }
    hosts.register(
        HostOp::new(
            "params_entries",
            vec![params_ty],
            HostType::List(Box::new(HostType::Tuple(vec![
                HostType::String,
                HostType::String,
            ]))),
            false,
            None,
            |args| {
                HostReply::Ready(params_resource(&args[0]).map(|params| {
                    HostValue::List(
                        params
                            .entries()
                            .into_iter()
                            .map(|(name, value)| {
                                HostValue::List(vec![
                                    HostValue::String(name),
                                    HostValue::String(value),
                                ])
                            })
                            .collect(),
                    )
                }))
            },
        )
        .with_receiver_method("URLSearchParams", "entries"),
    )?;
    Ok(())
}
fn text(value: &crate::HostValue) -> &str {
    let crate::HostValue::String(value) = value else {
        unreachable!("registered string argument")
    };
    value
}
fn optional_text(value: &crate::HostValue) -> Option<&str> {
    let crate::HostValue::Option(value) = value else {
        unreachable!("registered Option argument")
    };
    value.as_deref().map(text)
}
fn url_resource(value: &crate::HostValue) -> Result<&UrlObject> {
    let crate::HostValue::Handle(handle) = value else {
        return Err("URL host handle required".into());
    };
    handle
        .downcast_ref()
        .ok_or_else(|| "invalid URL resource".into())
}
fn params_resource(value: &crate::HostValue) -> Result<&SearchParams> {
    let crate::HostValue::Handle(handle) = value else {
        return Err("URLSearchParams host handle required".into());
    };
    handle
        .downcast_ref()
        .ok_or_else(|| "invalid URLSearchParams resource".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_resolves_and_normalizes_components() {
        let url = UrlObject::parse(
            "../world?q=hello world#hi",
            Some("https://User:Pass@EXAMPLE.com:443/a/b/"),
        )
        .unwrap();
        assert_eq!(
            url.href(),
            "https://User:Pass@example.com/a/world?q=hello%20world#hi"
        );
        assert_eq!(url.origin(), "https://example.com");
        assert_eq!(
            (url.protocol(), url.username(), url.password()),
            ("https:".into(), "User".into(), "Pass".into())
        );
        assert_eq!(
            (url.host(), url.hostname(), url.port()),
            ("example.com".into(), "example.com".into(), "".into())
        );
        assert_eq!(
            (url.pathname(), url.search(), url.hash()),
            ("/a/world".into(), "?q=hello%20world".into(), "#hi".into())
        );
        assert!(UrlObject::parse("/relative", None).is_err());
        assert!(UrlObject::parse("https://example.com", Some("bad base")).is_err());
        assert!(UrlObject::parse("https://[broken", None).is_err());
    }
    #[test]
    fn url_handles_unicode_ipv6_and_opaque_origins() {
        let url = UrlObject::parse("https://bücher.example/é", None).unwrap();
        assert_eq!(url.href(), "https://xn--bcher-kva.example/%C3%A9");
        let ip = UrlObject::parse("http://[::1]:8080/?#", None).unwrap();
        assert_eq!(ip.host(), "[::1]:8080");
        assert_eq!(ip.hostname(), "[::1]");
        assert_eq!(ip.port(), "8080");
        assert_eq!((ip.search(), ip.hash()), ("".into(), "".into()));
        let opaque = UrlObject::parse("mailto:hi@example.com", None).unwrap();
        assert_eq!(opaque.origin(), "null");
        assert_eq!(opaque.pathname(), "hi@example.com");
    }
    #[test]
    fn params_preserve_duplicates_order_and_empty_values() {
        let p = SearchParams::parse("?a=1&a=2&empty=&bare&x=a+b");
        assert_eq!(p.size(), 5);
        assert_eq!(p.get_all("a"), ["1", "2"]);
        assert_eq!(p.get("missing"), None);
        assert_eq!(p.get("bare"), Some("".into()));
        assert_eq!(p.get("x"), Some("a b".into()));
        p.set("a", "3");
        assert_eq!(p.serialize(), "a=3&empty=&bare=&x=a+b");
        p.append("a", "4");
        assert!(p.has("a", Some("4")));
        p.delete("a", Some("3"));
        assert_eq!(p.get_all("a"), ["4"]);
        p.delete("a", None);
        assert!(!p.has("a", None));
    }
    #[test]
    fn params_encoding_decoding_and_aliases() {
        let p = SearchParams::parse("plus=%2B&invalid=%FF&percent=%QZ");
        assert_eq!(p.get("plus"), Some("+".into()));
        assert_eq!(p.get("invalid"), Some("�".into()));
        assert_eq!(p.get("percent"), Some("%QZ".into()));
        let alias = p.clone();
        alias.append("tilde ~", "a+b");
        assert_eq!(
            p.serialize(),
            "plus=%2B&invalid=%EF%BF%BD&percent=%25QZ&tilde+%7E=a%2Bb"
        );
    }
    #[test]
    fn linked_params_share_canonical_url_state() {
        let url = UrlObject::parse("https://example.com/?a=b%20~#hash", None).unwrap();
        let params = url.search_params();
        let other = url.search_params();
        assert!(Rc::ptr_eq(&params, &other));
        params.sort();
        assert_eq!(url.href(), "https://example.com/?a=b+%7E#hash");
        other.append("a", "two");
        assert_eq!(params.get_all("a"), ["b ~", "two"]);
        params.delete("a", None);
        assert_eq!(url.href(), "https://example.com/#hash");
        drop(url);
        params.append("alive", "yes");
        assert_eq!(other.serialize(), "alive=yes");
    }
    #[test]
    fn params_sort_is_stable_utf16() {
        let p = SearchParams::from_pairs(vec![
            ("\u{E000}".into(), "first".into()),
            ("\u{10000}".into(), "second".into()),
            ("a".into(), "1".into()),
            ("a".into(), "2".into()),
        ]);
        p.sort();
        assert_eq!(p.keys(), ["a", "a", "\u{10000}", "\u{E000}"]);
        assert_eq!(p.values(), ["1", "2", "second", "first"]);
    }
}
