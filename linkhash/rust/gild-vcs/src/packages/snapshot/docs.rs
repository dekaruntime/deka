#[derive(Debug, Clone, Default)]
pub(super) struct DocComment {
    pub(super) summary: Option<String>,
    pub(super) description: Option<String>,
    pub(super) examples: Vec<String>,
    pub(super) typed_signature: Option<String>,
}

pub(super) fn parse_doc_comment_above(lines: &[&str], export_line: usize) -> Option<DocComment> {
    if export_line == 0 {
        return None;
    }

    let mut i = export_line;
    while i > 0 {
        i -= 1;
        let line = lines[i].trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("///") {
            let mut block = vec![lines[i].to_string()];
            let mut j = i;
            while j > 0 {
                let prev = lines[j - 1].trim_start();
                if prev.starts_with("///") {
                    j -= 1;
                    block.push(lines[j].to_string());
                    continue;
                }
                break;
            }
            block.reverse();
            return parse_slash_doc_block(&block);
        }

        if !line.ends_with("*/") {
            return None;
        }

        let mut block = vec![lines[i].to_string()];
        let mut j = i;
        let found_start = line.starts_with("/**");
        while !found_start {
            if j == 0 {
                return None;
            }
            j -= 1;
            let current = lines[j];
            block.push(current.to_string());
            if current.trim_start().starts_with("/**") {
                break;
            }
        }
        block.reverse();
        return parse_doc_block(&block);
    }
    None
}

fn parse_doc_block(block: &[String]) -> Option<DocComment> {
    if block.is_empty() {
        return None;
    }

    let mut lines = Vec::new();
    for (idx, raw) in block.iter().enumerate() {
        let mut line = raw.trim().to_string();
        if idx == 0 {
            line = line.trim_start_matches("/**").trim().to_string();
        }
        if idx + 1 == block.len() {
            line = line.trim_end_matches("*/").trim().to_string();
        }
        line = line.trim_start_matches('*').trim().to_string();
        if !line.is_empty() {
            lines.push(line);
        }
    }

    if lines.is_empty() {
        return None;
    }

    let mut summary = None;
    let mut description_lines = Vec::new();
    let mut examples = Vec::new();
    for line in lines {
        if line.starts_with("@example") {
            let example = line.trim_start_matches("@example").trim().to_string();
            if !example.is_empty() {
                examples.push(example);
            }
            continue;
        }
        if line.starts_with('@') {
            continue;
        }
        if summary.is_none() {
            summary = Some(line.clone());
        }
        description_lines.push(line);
    }

    let description = if description_lines.is_empty() {
        None
    } else {
        Some(description_lines.join("\n"))
    };

    Some(DocComment {
        summary,
        description,
        examples,
        typed_signature: None,
    })
}

fn parse_slash_doc_block(block: &[String]) -> Option<DocComment> {
    if block.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    for raw in block {
        let mut line = raw.trim_start().to_string();
        line = line.trim_start_matches("///").trim().to_string();
        if !line.is_empty() {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        return None;
    }

    let mut summary = None;
    let mut description = None;
    let mut examples = Vec::new();
    let mut typed_signature = None;

    if let Some(function_name) = extract_xml_attr(&lines, "Function", "name") {
        let params = extract_parameter_signature_parts(&lines);
        let return_type = extract_xml_attr(&lines, "ReturnType", "type");
        let mut sig = format!("{}(", function_name);
        sig.push_str(&params.join(", "));
        sig.push(')');
        if let Some(rt) = return_type {
            if !rt.trim().is_empty() {
                sig.push_str(": ");
                sig.push_str(rt.trim());
            }
        }
        typed_signature = Some(sig);
    }

    if let Some(desc) = extract_xml_tag(&lines, "Description") {
        let clean = desc.trim().to_string();
        if !clean.is_empty() {
            summary = Some(clean.clone());
            description = Some(clean);
        }
    }

    for line in &lines {
        if line.starts_with("@example") {
            let ex = line.trim_start_matches("@example").trim().to_string();
            if !ex.is_empty() {
                examples.push(ex);
            }
        }
    }

    if summary.is_none() {
        for line in &lines {
            if line.starts_with("docid:") || line.starts_with('<') {
                continue;
            }
            let clean = line.trim().to_string();
            if !clean.is_empty() {
                summary = Some(clean.clone());
                description = Some(clean);
                break;
            }
        }
    }

    if summary.is_none() && description.is_none() && examples.is_empty() {
        return None;
    }

    Some(DocComment {
        summary,
        description,
        examples,
        typed_signature,
    })
}

fn extract_xml_attr(lines: &[String], tag: &str, attr: &str) -> Option<String> {
    let tag_open = format!("<{}", tag);
    let needle = format!("{}=\"", attr);
    for line in lines {
        if !line.contains(&tag_open) {
            continue;
        }
        let start = line.find(&needle)?;
        let rest = &line[start + needle.len()..];
        let end = rest.find('"')?;
        let value = rest[..end].trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn extract_parameter_signature_parts(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        if !line.contains("<Parameter ") {
            continue;
        }
        let name = extract_attr_from_line(line, "name").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let param_type = extract_attr_from_line(line, "type").unwrap_or_default();
        if param_type.is_empty() {
            out.push(name);
        } else {
            out.push(format!("{} {}", name, param_type));
        }
    }
    out
}

fn extract_attr_from_line(line: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = line.find(&needle)?;
    let rest = &line[start + needle.len()..];
    let end = rest.find('"')?;
    let value = rest[..end].trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn extract_xml_tag(lines: &[String], tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let mut collecting = false;
    let mut out = Vec::new();
    for line in lines {
        if !collecting {
            if let Some(start) = line.find(&open) {
                let rest = &line[start + open.len()..];
                if let Some(end) = rest.find(&close) {
                    let inner = rest[..end].trim().to_string();
                    if !inner.is_empty() {
                        return Some(inner);
                    }
                    continue;
                }
                collecting = true;
                let first = rest.trim();
                if !first.is_empty() {
                    out.push(first.to_string());
                }
            }
            continue;
        }

        if let Some(end) = line.find(&close) {
            let part = line[..end].trim();
            if !part.is_empty() {
                out.push(part.to_string());
            }
            break;
        }
        let t = line.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("\n"))
    }
}
