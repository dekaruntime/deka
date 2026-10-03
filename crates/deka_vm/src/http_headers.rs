//! HTTP header storage shared by native Headers and later Request/Response hosts.
//! Names, normalization and duplicate handling follow the Fetch header-list rules.
use crate::Result;
use std::collections::BTreeMap;

/// A header list keeps duplicate values in insertion order. Iteration sorts
/// names and combines duplicates, except Set-Cookie which stays separate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeaderList {
    values: BTreeMap<String, Vec<String>>,
}

fn name(name: &str) -> Result<String> {
    if name.is_empty()
        || !name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
    {
        return Err("invalid header name".into());
    }
    Ok(name.to_ascii_lowercase())
}

fn value(value: &str) -> Result<String> {
    // Web header inputs are byte strings, not arbitrary Unicode. Keep the
    // isomorphic Latin-1 characters and trim only HTTP whitespace, not Unicode.
    if value.chars().any(|ch| u32::from(ch) > 255) {
        return Err("header value must contain only byte-string characters".into());
    }
    let value = value.trim_matches(['\t', '\n', '\r', ' ']);
    if value.chars().any(|ch| matches!(ch, '\0' | '\n' | '\r')) {
        return Err("invalid header value".into());
    }
    Ok(value.into())
}

impl HeaderList {
    /// Construct from the familiar sequence of name/value pairs. Validation
    /// happens before publishing the resource, so failures expose no partial list.
    pub fn from_pairs(pairs: &[Vec<String>]) -> Result<Self> {
        let mut headers = Self::default();
        for pair in pairs {
            let [name, value] = pair.as_slice() else {
                return Err("header initializer entries must have exactly two strings".into());
            };
            headers.append(name, value)?;
        }
        Ok(headers)
    }
    pub fn append(&mut self, key: &str, text: &str) -> Result<()> {
        let key = name(key)?;
        let text = value(text)?;
        self.values.entry(key).or_default().push(text);
        Ok(())
    }
    pub fn set(&mut self, key: &str, text: &str) -> Result<()> {
        let key = name(key)?;
        let text = value(text)?;
        self.values.insert(key, vec![text]);
        Ok(())
    }
    pub fn delete(&mut self, key: &str) -> Result<()> {
        self.values.remove(&name(key)?);
        Ok(())
    }
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.values.get(&name(key)?).map(|values| values.join(", ")))
    }
    pub fn has(&self, key: &str) -> Result<bool> {
        Ok(self.values.contains_key(&name(key)?))
    }
    pub fn get_set_cookie(&self) -> Vec<String> {
        self.values.get("set-cookie").cloned().unwrap_or_default()
    }
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut entries = Vec::new();
        for (name, values) in &self.values {
            if name == "set-cookie" {
                entries.extend(values.iter().map(|value| (name.clone(), value.clone())));
            } else {
                entries.push((name.clone(), values.join(", ")));
            }
        }
        entries
    }
    pub fn keys(&self) -> Vec<String> {
        self.entries().into_iter().map(|(name, _)| name).collect()
    }
    pub fn values(&self) -> Vec<String> {
        self.entries().into_iter().map(|(_, value)| value).collect()
    }
    /// Preserve duplicate fields when handing bytes to the HTTP transport.
    /// Deka strings remain UTF-8 internally; HTTP ByteString values use Latin-1.
    pub fn wire_entries(&self) -> Vec<(String, Vec<u8>)> {
        self.values
            .iter()
            .flat_map(|(name, values)| {
                values.iter().map(|value| {
                    (
                        name.clone(),
                        value.chars().map(|ch| u32::from(ch) as u8).collect(),
                    )
                })
            })
            .collect()
    }
}

/// The same Rust allocation backs every language alias. Request/Response will
/// use this header-list representation rather than building another header store.
pub(crate) fn handle(list: HeaderList) -> crate::HostValue {
    crate::HostValue::Handle(crate::HostHandle::new(
        "Headers",
        std::cell::RefCell::new(list),
    ))
}

pub fn register(hosts: &mut crate::Hosts) -> Result<()> {
    use crate::{HostOp, HostReply, HostType, HostValue};
    let pairs = || {
        HostType::List(Box::new(HostType::Tuple(vec![
            HostType::String,
            HostType::String,
        ])))
    };
    hosts.register(
        HostOp::new(
            "Headers",
            vec![pairs()],
            HostType::Handle("Headers".into()),
            false,
            |args| {
                let [HostValue::List(entries)] = args.as_slice() else {
                    unreachable!("checked Headers initializer")
                };
                let entries: Vec<Vec<String>> = entries
                    .iter()
                    .map(|entry| {
                        let HostValue::List(pair) = entry else {
                            unreachable!("checked header tuple")
                        };
                        pair.iter()
                            .map(|item| {
                                let HostValue::String(text) = item else {
                                    unreachable!("checked header string")
                                };
                                text.clone()
                            })
                            .collect()
                    })
                    .collect();
                HostReply::Ready(HeaderList::from_pairs(&entries).map(handle))
            },
        )
        .with_defaults(vec![HostValue::List(vec![])])
        .with_global_binding()
        .with_result_channel(),
    )?;
    for (method, arguments, result, result_channel) in [
        (
            "append",
            vec![HostType::String, HostType::String],
            HostType::Unit,
            true,
        ),
        (
            "set",
            vec![HostType::String, HostType::String],
            HostType::Unit,
            true,
        ),
        ("delete", vec![HostType::String], HostType::Unit, true),
        (
            "get",
            vec![HostType::String],
            HostType::Option(Box::new(HostType::String)),
            true,
        ),
        ("has", vec![HostType::String], HostType::Bool, true),
        ("getSetCookie", vec![], HostType::Strings, false),
        ("entries", vec![], pairs(), false),
        ("keys", vec![], HostType::Strings, false),
        ("values", vec![], HostType::Strings, false),
    ] {
        let mut args = vec![HostType::Handle("Headers".into())];
        args.extend(arguments);
        let op = HostOp::new(
            &format!("__headers_{method}"),
            args,
            result,
            false,
            move |args| {
                let HostValue::Handle(resource) = &args[0] else {
                    unreachable!("checked Headers receiver")
                };
                let Some(list) = resource.downcast_ref::<std::cell::RefCell<HeaderList>>() else {
                    return HostReply::Ready(Err("invalid Headers resource".into()));
                };
                let text = |index: usize| {
                    let HostValue::String(value) = &args[index] else {
                        unreachable!("checked Headers argument")
                    };
                    value.as_str()
                };
                let result = match method {
                    "append" => list
                        .borrow_mut()
                        .append(text(1), text(2))
                        .map(|()| HostValue::Unit),
                    "set" => list
                        .borrow_mut()
                        .set(text(1), text(2))
                        .map(|()| HostValue::Unit),
                    "delete" => list.borrow_mut().delete(text(1)).map(|()| HostValue::Unit),
                    "get" => list.borrow().get(text(1)).map(|value| {
                        HostValue::Option(value.map(|text| Box::new(HostValue::String(text))))
                    }),
                    "has" => list.borrow().has(text(1)).map(HostValue::Bool),
                    "getSetCookie" => Ok(HostValue::Strings(list.borrow().get_set_cookie())),
                    "keys" => Ok(HostValue::Strings(list.borrow().keys())),
                    "values" => Ok(HostValue::Strings(list.borrow().values())),
                    "entries" => Ok(HostValue::List(
                        list.borrow()
                            .entries()
                            .into_iter()
                            .map(|(key, value)| {
                                HostValue::List(vec![
                                    HostValue::String(key),
                                    HostValue::String(value),
                                ])
                            })
                            .collect(),
                    )),
                    _ => unreachable!("registered Headers method"),
                };
                HostReply::Ready(result)
            },
        )
        .with_receiver_method("Headers", method);
        hosts.register(if result_channel {
            op.with_result_channel()
        } else {
            op
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_is_case_insensitive_and_preserves_empty_vs_missing() {
        let mut h = HeaderList::default();
        h.append("Content-Type", "\r\n  application/json\t")
            .unwrap();
        h.append("X-Empty", "").unwrap();
        assert_eq!(
            h.get("CONTENT-type").unwrap(),
            Some("application/json".into())
        );
        assert_eq!(h.get("x-empty").unwrap(), Some(String::new()));
        assert_eq!(h.get("missing").unwrap(), None);
        assert!(h.has("content-TYPE").unwrap());
        assert!(!h.has("missing").unwrap());
    }
    #[test]
    fn duplicate_values_and_cookies_have_distinct_iteration_rules() {
        let mut h = HeaderList::default();
        h.append("Z", "last").unwrap();
        h.append("X", "one").unwrap();
        h.append("x", "two").unwrap();
        h.append("Set-Cookie", "a=1; Expires=Wed, 21 Oct 2030 07:28:00 GMT")
            .unwrap();
        h.append("SET-cookie", "b=2").unwrap();
        assert_eq!(h.get("x").unwrap(), Some("one, two".into()));
        let cookies = h.get_set_cookie();
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[1], "b=2");
        let entries = h.entries();
        assert_eq!(entries[0], ("set-cookie".into(), cookies[0].clone()));
        assert_eq!(entries[1], ("set-cookie".into(), "b=2".into()));
        assert_eq!(entries[2], ("x".into(), "one, two".into()));
        assert_eq!(entries[3], ("z".into(), "last".into()));
        assert_eq!(h.keys(), ["set-cookie", "set-cookie", "x", "z"]);
        assert_eq!(
            h.values(),
            entries.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>()
        );
        assert!(HeaderList::default().get_set_cookie().is_empty());
    }
    #[test]
    fn set_replaces_all_duplicates_and_delete_removes_only_its_name() {
        let mut h = HeaderList::default();
        h.append("x", "one").unwrap();
        h.append("x", "two").unwrap();
        h.append("y", "other").unwrap();
        h.set("X", "replacement").unwrap();
        assert_eq!(h.get("x").unwrap(), Some("replacement".into()));
        assert_eq!(h.entries().len(), 2);
        h.delete("X").unwrap();
        h.delete("missing").unwrap();
        assert_eq!(h.get("x").unwrap(), None);
        assert_eq!(h.get("y").unwrap(), Some("other".into()));
    }
    #[test]
    fn bad_names_values_and_pair_arity_fail_without_mutating_the_list() {
        let mut h = HeaderList::default();
        h.append("x", "before").unwrap();
        let before = h.clone();
        for key in ["", "bad name", "colon:", "é", "x\n", "x\0"] {
            assert!(h.append(key, "v").is_err());
            assert!(h.set(key, "v").is_err());
            assert!(h.delete(key).is_err());
            assert!(h.get(key).is_err());
            assert!(h.has(key).is_err());
            assert_eq!(h, before);
        }
        for value in ["a\nb", "a\rb", "a\0b", "🙂"] {
            assert!(h.append("x", value).is_err());
            assert!(h.set("x", value).is_err());
            assert_eq!(h, before);
        }
        for pair in [
            vec![],
            vec!["x".into()],
            vec!["x".into(), "v".into(), "extra".into()],
        ] {
            assert!(HeaderList::from_pairs(&[pair]).is_err());
        }
        assert!(
            HeaderList::from_pairs(&[
                vec!["x".into(), "ok".into()],
                vec!["bad name".into(), "no".into()]
            ])
            .is_err()
        );
    }
    #[test]
    fn normalization_is_http_whitespace_and_transport_bytes_are_isomorphic() {
        let mut h = HeaderList::default();
        h.append("X-Latin", "\t café\r\n").unwrap();
        h.append("x-latin", "\u{a0}keep\u{a0}").unwrap();
        assert_eq!(
            h.get("x-latin").unwrap(),
            Some("café, \u{a0}keep\u{a0}".into())
        );
        assert_eq!(
            h.wire_entries(),
            vec![
                ("x-latin".into(), vec![b'c', b'a', b'f', 0xe9]),
                ("x-latin".into(), vec![0xa0, b'k', b'e', b'e', b'p', 0xa0])
            ]
        );
        assert!(h.append("x", "\u{100}").is_err());
    }
    #[test]
    fn initializer_keeps_duplicates_and_valid_token_punctuation() {
        let h = HeaderList::from_pairs(&[
            vec!["X!#$%&'*+-.^_`|~".into(), "first".into()],
            vec!["x!#$%&'*+-.^_`|~".into(), "second".into()],
        ])
        .unwrap();
        assert_eq!(
            h.get("X!#$%&'*+-.^_`|~").unwrap(),
            Some("first, second".into())
        );
    }
}
