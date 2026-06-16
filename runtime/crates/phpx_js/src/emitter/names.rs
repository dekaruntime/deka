use super::*;

impl<'a> JsSubsetEmitter<'a> {
    pub(super) fn token_name(&self, tok: &php_rs::parser::lexer::token::Token) -> String {
        self.sanitize_name(self.token_text(tok).as_str())
    }

    pub(super) fn span_name(&self, span: php_rs::parser::span::Span) -> String {
        let text = String::from_utf8_lossy(self.span_bytes(span)).to_string();
        self.sanitize_name(&text)
    }

    pub(super) fn sanitize_name(&self, raw: &str) -> String {
        let mut name = raw
            .trim()
            .trim_start_matches('$')
            .trim_start_matches('\\')
            .replace('\\', "_");
        if name == "this" {
            return "this".to_string();
        }
        if name.is_empty() {
            name = "_".to_string();
        }
        if is_js_reserved_word(&name) {
            name.push('_');
        }
        name
    }

    pub(super) fn token_text(&self, tok: &php_rs::parser::lexer::token::Token) -> String {
        String::from_utf8_lossy(tok.text(self.source)).to_string()
    }

    pub(super) fn span_bytes(&self, span: php_rs::parser::span::Span) -> &'a [u8] {
        &self.source[span.start..span.end]
    }

    pub(super) fn encode_php_string_literal(&self, value: &[u8]) -> String {
        let mut bytes: &[u8] = value;
        let mut quote = None;
        if bytes.len() >= 2 {
            let first = bytes[0];
            let last = bytes[bytes.len() - 1];
            if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
                quote = Some(first);
                bytes = &bytes[1..bytes.len() - 1];
            }
        }
        let raw = String::from_utf8_lossy(bytes).to_string();
        let decoded = match quote {
            Some(b'\'') => unescape_php_single(&raw),
            Some(b'"') => unescape_php_double(&raw),
            _ => raw,
        };
        json_string(&decoded)
    }

    pub(super) fn normalize_jsx_text(&self, span: php_rs::parser::span::Span) -> Option<String> {
        let raw = String::from_utf8_lossy(self.span_bytes(span)).to_string();
        if raw.chars().all(|c| c.is_whitespace()) {
            if raw.contains('\n') || raw.contains('\r') {
                None
            } else {
                Some(" ".to_string())
            }
        } else {
            Some(raw)
        }
    }
}
