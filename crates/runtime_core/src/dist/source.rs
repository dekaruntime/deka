//! Shared `.ds`/`.dsx` source-text scanning: comment stripping and `export fn`
//! detection. Pure text in → facts out; nothing here touches the compiler.
pub(super) fn exports_fn_named(src: &str, name: &str) -> bool {
    export_fn_needles(name)
        .into_iter()
        .any(|needle| contains_export_fn(src, &needle))
        || export_list_contains(src, name)
}

fn export_fn_needles(name: &str) -> Vec<String> {
    vec![
        format!("export fn {name}"),
        format!("export async fn {name}"),
        format!("export function {name}"),
        format!("export async function {name}"),
    ]
}

fn contains_export_fn(src: &str, needle: &str) -> bool {
    let mut rest = src;
    while let Some(at) = rest.find(needle) {
        let after = &rest[at + needle.len()..];
        let boundary = match after.chars().next() {
            None => true,
            Some(c) => !c.is_ascii_alphanumeric() && c != '_',
        };
        if boundary {
            return true;
        }
        rest = &rest[at + 1..];
    }
    false
}

fn export_list_contains(src: &str, name: &str) -> bool {
    let mut rest = src;
    while let Some(at) = rest.find("export {") {
        let after = &rest[at + "export {".len()..];
        let Some(end) = after.find('}') else {
            break;
        };
        for part in after[..end].split(',') {
            let ident = part
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('|');
            if ident == name {
                return true;
            }
        }
        rest = &after[end.saturating_add(1)..];
    }
    false
}

pub(super) fn strip_ds_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = i.saturating_add(2).min(bytes.len());
            continue;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            out.push(quote as char);
            i += 1;
            while i < bytes.len() {
                out.push(bytes[i] as char);
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    if i < bytes.len() {
                        out.push(bytes[i] as char);
                    }
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ds_comments_table() {
        let cases: &[(&str, &str)] = &[
            // Line and block comments are removed...
            ("let a = 1 // gone\nlet b = 2", "let a = 1 \nlet b = 2"),
            ("let a = 1 /* gone */ + 2", "let a = 1  + 2"),
            // ...but comment syntax inside string literals survives.
            ("let url = \"http://x\" // c\n", "let url = \"http://x\" \n"),
            (
                "let s = '/* not a comment */'",
                "let s = '/* not a comment */'",
            ),
            // Escaped quotes don't end the string early.
            ("let s = \"a\\\"//b\"", "let s = \"a\\\"//b\""),
        ];
        for (src, expected) in cases {
            assert_eq!(strip_ds_comments(src), *expected, "input {src:?}");
        }
    }

    #[test]
    fn export_detection_table() {
        let cases: &[(&str, &str, bool)] = &[
            ("export fn GET() {}", "GET", true),
            ("export async fn GET() {}", "GET", true),
            ("export function GET() {}", "GET", true),
            ("export fn GETTER() {}", "GET", false),
            ("fn GET() {}\nexport { GET }", "GET", true),
            ("fn GET() {}", "GET", false),
        ];
        for (src, name, expected) in cases {
            assert_eq!(
                exports_fn_named(src, name),
                *expected,
                "exports_fn_named({src:?}, {name:?})"
            );
        }
    }
}
