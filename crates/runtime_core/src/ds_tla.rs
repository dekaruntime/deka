//! Top-level `await` without the compiler crates.
//!
//! `await` inside `fn` / `function` bodies is not top-level. `await` in
//! module-scope statements (including `for` / `if` / `match` blocks) is.

pub fn has_top_level_await(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut i = 0;
    // true = this `{` opened a function body
    let mut braces: Vec<bool> = Vec::new();
    let mut expecting_fn_body = false;
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
            b'`' => i = skip_template(bytes, i),
            b'f' if is_word(bytes, i, b"fn") || is_word(bytes, i, b"function") => {
                expecting_fn_body = true;
                i += if bytes.get(i + 1) == Some(&b'n') {
                    2
                } else {
                    8
                };
            }
            b'{' => {
                braces.push(expecting_fn_body);
                expecting_fn_body = false;
                i += 1;
            }
            b'}' => {
                braces.pop();
                i += 1;
            }
            b'a' if is_word(bytes, i, b"await") => {
                if !braces.iter().any(|is_fn| *is_fn) {
                    return true;
                }
                i += 5;
            }
            _ => i += 1,
        }
    }
    false
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

fn skip_string(bytes: &[u8], i: usize) -> usize {
    let quote = bytes[i];
    let mut j = i + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' {
            j = j.saturating_add(2);
            continue;
        }
        if bytes[j] == quote {
            return j + 1;
        }
        j += 1;
    }
    bytes.len()
}

fn skip_template(bytes: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' {
            j = j.saturating_add(2);
            continue;
        }
        if bytes[j] == b'`' {
            return j + 1;
        }
        j += 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::has_top_level_await;

    #[test]
    fn top_level_await_detected() {
        assert!(has_top_level_await(
            "async fn main() Promise<number> { return 1 } const n = await main();"
        ));
    }

    #[test]
    fn await_inside_function_is_not_top_level() {
        assert!(!has_top_level_await(
            "async fn main() Promise<number> { return await other() }"
        ));
    }

    #[test]
    fn await_inside_closure_is_not_top_level() {
        assert!(!has_top_level_await(
            "const f = fn () Promise<number> { return await other() }"
        ));
    }

    #[test]
    fn await_inside_top_level_for_loop_is_top_level() {
        assert!(has_top_level_await(
            "async fn work() Promise<number> { return 1 } for (let i = 0; i < 3; i = i + 1) { await work() }"
        ));
    }

    #[test]
    fn no_await_means_no_top_level_await() {
        assert!(!has_top_level_await(
            "const x = 1; fn f() number { return x }"
        ));
    }

    #[test]
    fn skips_await_in_comments_and_strings() {
        assert!(!has_top_level_await(
            "// await foo()\nconst s = \"await bar\"\nfn f() { return 1 }\n"
        ));
    }
}
