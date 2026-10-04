use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl Span {
    pub(crate) fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

pub(crate) struct LineIndex {
    pub(crate) line_starts: Vec<usize>,
    source: String,
}

impl LineIndex {
    pub(crate) fn new(source: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (idx, byte) in source.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        Self {
            line_starts,
            source: source.to_owned(),
        }
    }

    pub(crate) fn offset_to_position(&self, offset: usize) -> Position {
        let mut offset = offset.min(self.source.len());
        while !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        let start = self.line_starts[line];
        Position {
            line: line as u32,
            character: self.source[start..offset].encode_utf16().count() as u32,
        }
    }
    pub(crate) fn position_to_offset(&self, position: Position) -> Option<usize> {
        let start = *self.line_starts.get(position.line as usize)?;
        let text = self.source[start..].split('\n').next()?;
        let mut units = 0;
        for (byte, ch) in text.char_indices() {
            if units == position.character {
                return Some(start + byte);
            }
            units += ch.len_utf16() as u32;
            if units > position.character {
                return None;
            }
        }
        (units == position.character).then_some(start + text.len())
    }
}

pub(crate) fn span_to_range(span: Span, line_index: &LineIndex) -> Range {
    Range {
        start: line_index.offset_to_position(span.start),
        end: line_index.offset_to_position(span.end),
    }
}

pub(crate) fn word_at_offset(source: &[u8], offset: usize) -> Option<String> {
    let span = word_span_at_offset(source, offset)?;
    Some(String::from_utf8_lossy(&source[span.start..span.end]).to_string())
}

/// The byte span of the identifier under `offset`, for hover ranges.
pub(crate) fn word_span_at_offset(source: &[u8], offset: usize) -> Option<Span> {
    let bytes = source;
    if offset >= bytes.len() {
        return None;
    }
    let mut start = offset;
    let mut end = offset;
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_ident_char(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let word = &bytes[start..end];
    if word.iter().all(|b| b.is_ascii_whitespace()) {
        return None;
    }
    Some(Span::new(start, end))
}

pub(crate) fn is_ident_char(byte: u8) -> bool {
    byte == b'$'
        || byte == b'_'
        || byte.is_ascii_digit()
        || byte.is_ascii_lowercase()
        || byte.is_ascii_uppercase()
        || byte == b'\\'
}

pub(crate) fn find_word_occurrences(source: &[u8], word: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let needle = word.as_bytes();
    if needle.is_empty() || needle.len() > source.len() {
        return spans;
    }
    let mut offset = 0usize;
    while offset + needle.len() <= source.len() {
        let Some(pos) = source[offset..]
            .windows(needle.len())
            .position(|window| window == needle)
        else {
            break;
        };
        let start = offset + pos;
        let end = start + needle.len();
        let left_ok = start == 0 || !is_ident_char(source[start - 1]);
        let right_ok = end >= source.len() || !is_ident_char(source[end]);
        if left_ok && right_ok {
            spans.push(Span::new(start, end));
        }
        offset = end;
    }
    spans
}

pub(crate) fn hover_from_import(source: &str, offset: usize) -> Option<String> {
    let imports = parse_imports(source);
    for import in imports {
        if import.span.start <= offset && offset < import.span.end {
            let line = if import.imported == "default" {
                format!("import {} from '{}'", import.local, import.from)
            } else {
                format!("import {{ {} }} from '{}'", import.local, import.from)
            };
            return Some(format!("```dekascript\n{}\n```", line));
        }
    }
    None
}

pub(crate) fn hover_for_annotation(source: &str, offset: usize) -> Option<String> {
    let name = annotation_name_at_offset(source, offset)?;
    let (_, detail) = annotation_catalog()
        .into_iter()
        .find(|(label, _)| *label == name.as_str())?;
    Some(format!("```dekascript\n@{}\n```\n{}", name, detail))
}

pub(crate) fn annotation_name_at_offset(source: &str, offset: usize) -> Option<String> {
    let bytes = source.as_bytes();
    if offset > bytes.len() {
        return None;
    }
    let mut start = offset.min(bytes.len());
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = offset.min(bytes.len());
    while end < bytes.len() && is_ident_char(bytes[end]) {
        end += 1;
    }
    if start >= end {
        return None;
    }
    if start == 0 || bytes[start - 1] != b'@' {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[start..end]).to_string())
}
