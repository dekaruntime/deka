//! Import specifiers from DekaScript source without the compiler crates.
//!
//! Used by the project gate and import maps. Matches `import "path"`,
//! `import { ... } from "path"`, and `export { ... } from "path"`.

pub fn paths(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = i.saturating_add(2);
            }
            b'"' | b'\'' => i = skip_string(bytes, i),
            b'i' if is_word(bytes, i, b"import") => {
                i += 6;
                i = skip_ws(bytes, i);
                if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                    if let Some((path, next)) = take_string(bytes, i) {
                        out.push(path);
                        i = next;
                        continue;
                    }
                }
            }
            b'f' if is_word(bytes, i, b"from") => {
                i += 4;
                i = skip_ws(bytes, i);
                if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                    if let Some((path, next)) = take_string(bytes, i) {
                        out.push(path);
                        i = next;
                        continue;
                    }
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn is_word(bytes: &[u8], i: usize, word: &[u8]) -> bool {
    let end = i + word.len();
    if end > bytes.len() || &bytes[i..end] != word {
        return false;
    }
    let before_ok = i == 0 || !is_ident(bytes[i - 1]);
    let after_ok = end == bytes.len() || !is_ident(bytes[end]);
    before_ok && after_ok
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn skip_string(bytes: &[u8], i: usize) -> usize {
    take_string(bytes, i).map(|(_, next)| next).unwrap_or(i + 1)
}

fn take_string(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let quote = *bytes.get(i)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' {
            j = j.saturating_add(2);
            continue;
        }
        if bytes[j] == quote {
            let path = String::from_utf8_lossy(&bytes[i + 1..j]).into_owned();
            return Some((path, j + 1));
        }
        j += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::paths;

    #[test]
    fn named_and_side_effect_imports() {
        let src = r#"
import { echo } from "io"
import "./peer.ds"
export { Point } from "./a.ds"
const from = "not-an-import"
"#;
        assert_eq!(paths(src), vec!["io", "./peer.ds", "./a.ds"]);
    }

    #[test]
    fn skips_comments_and_strings() {
        let src = r#"
// import { x } from "nope"
/* import { y } from "nope2" */
const s = "import { z } from \"nope3\""
import { real } from "@deka/greeter"
"#;
        assert_eq!(paths(src), vec!["@deka/greeter"]);
    }

    #[test]
    fn multiline_named_import() {
        let src = "import {\n  foo,\n  bar\n} from \"./mod.ds\"\n";
        assert_eq!(paths(src), vec!["./mod.ds"]);
    }
}
