//! Decode APS55 findings with the same position grammar as the native CLI.
use deka_vm::compiler::diagnostic_position;

pub(super) fn parse(stderr: &str) -> Vec<String> {
    let native: Vec<String> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("[check] "))
        .filter_map(diagnostic_position)
        .map(|(_, _, message)| message.trim())
        .filter(|message| !message.is_empty())
        .map(str::to_owned)
        .collect();
    if !native.is_empty() {
        // Source headers and context are not findings. Preserve source order;
        // positions remain available in the unchanged captured stderr.
        return native;
    }
    let mut diagnostics = Vec::new();
    for line in stderr.lines() {
        let line = if let Some(body) = line.strip_prefix("[check] ") {
            // A source header or source-code context may itself contain ^.
            // Only an actual legacy caret finding participates in fallback.
            let body = body.trim_start();
            if !body.starts_with('^') {
                continue;
            }
            body
        } else {
            line
        };
        if let Some((_, message)) = line.split_once('^') {
            let message = message.trim();
            if !message.is_empty() {
                diagnostics.push(message.to_string());
            }
        }
    }
    if diagnostics.is_empty() {
        if let Some(first) = stderr.lines().map(str::trim).find(|line| {
            !line.is_empty()
                && !line.starts_with('[')
                && !line.starts_with("Validation")
                && !line.starts_with('❌')
        }) {
            diagnostics.push(first.to_string());
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_findings_keep_order_without_headers_context_or_message_truncation() {
        assert_eq!(
            parse(
                "[check] ./main.ds\n[check] /sources/name: part.ds\n[check] 2:6: expected type: string ^ number\n[check] 3:6: second finding\n[check]   3 | context ^ not a finding\n"
            ),
            ["expected type: string ^ number", "second finding"]
        );
    }
    #[test]
    fn header_only_output_is_not_a_diagnostic() {
        assert!(
            parse("[check] ./main.ds\n[check] /sources/helper^name.ds\n[security] context\n")
                .is_empty()
        );
    }
    #[test]
    fn caret_and_plain_native_fallbacks_remain_compatible() {
        assert_eq!(
            parse("source\n    ^ old first\n    ^ old second\n"),
            ["old first", "old second"]
        );
        assert_eq!(
            parse("[security] ignored\ntype mismatch at 3:1\n"),
            ["type mismatch at 3:1"]
        );
    }
}
