pub(super) fn collect_statement(lines: &[&str], start: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut i = start;
    while i < lines.len() {
        out.push(lines[i]);
        let t = lines[i].trim_end();
        if t.ends_with(';') || t.ends_with('{') {
            i += 1;
            break;
        }
        i += 1;
    }
    (out.join("\n"), i)
}

pub(super) fn collect_brace_block(lines: &[&str], start: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut seen_open = false;
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        out.push(line);
        for ch in line.chars() {
            if ch == '{' {
                depth += 1;
                seen_open = true;
            } else if ch == '}' {
                depth -= 1;
            }
        }
        i += 1;
        if seen_open && depth <= 0 {
            break;
        }
    }
    (out.join("\n"), i)
}

pub(super) fn extract_named_kind(trimmed: &str) -> Option<(String, String)> {
    if trimmed.starts_with("export struct ") {
        extract_name_after(trimmed, "export struct").map(|name| ("struct".to_string(), name))
    } else if trimmed.starts_with("export enum ") {
        extract_name_after(trimmed, "export enum").map(|name| ("enum".to_string(), name))
    } else {
        None
    }
}

pub(super) fn extract_name_after(line: &str, prefix: &str) -> Option<String> {
    let tail = line.strip_prefix(prefix)?.trim_start();
    let mut name = String::new();
    for ch in tail.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
        } else {
            break;
        }
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

pub(in crate::packages) fn parse_reexport_names(decl: &str) -> Vec<String> {
    let open = match decl.find('{') {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let close = match decl[open + 1..].find('}') {
        Some(idx) => open + 1 + idx,
        None => return Vec::new(),
    };

    decl[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter_map(|item| {
            if let Some((_, alias)) = item.split_once(" as ") {
                let v = alias.trim();
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                }
            } else {
                Some(item.to_string())
            }
        })
        .collect()
}

pub(super) fn normalize_ws(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}
