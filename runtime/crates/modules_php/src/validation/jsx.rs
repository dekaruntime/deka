use std::collections::HashSet;

use bumpalo::Bump;
use php_rs::parser::ast::BinaryOp;
use php_rs::parser::ast::visitor::{Visitor, walk_expr};
use php_rs::parser::ast::{Expr, ExprId, JsxAttribute, JsxChild, Name, Program, Stmt};
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use php_rs::parser::span::Span;
use php_rs::phpx::typeck::check_program_with_path;

use super::{ErrorKind, Severity, ValidationError};
use crate::validation::imports::{
    consume_comment_line, find_column, frontmatter_bounds, parse_import_line, strip_php_tags_inline,
};

pub fn validate_jsx_syntax(program: &Program, source: &str) -> Vec<ValidationError> {
    let mut validator = JsxSyntaxValidator {
        source,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

pub fn validate_jsx_expressions(program: &Program, source: &str) -> Vec<ValidationError> {
    let mut validator = JsxExpressionValidator {
        source,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

pub fn validate_components(program: &Program, source: &str) -> Vec<ValidationError> {
    let mut known_components = collect_function_names(program, source);
    let async_components = collect_async_function_names(program, source);
    known_components.extend(collect_imported_names(source));
    known_components.insert("Link".to_string());
    known_components.insert("ContextProvider".to_string());
    known_components.insert("Suspense".to_string());

    let mut validator = JsxComponentValidator {
        source,
        known_components,
        async_components,
        suspense_depth: 0,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

pub fn validate_frontmatter(source: &str, file_path: &str) -> Vec<ValidationError> {
    let lines: Vec<&str> = source.lines().collect();
    let mut errors = Vec::new();
    let mut saw_frontmatter = false;

    let mut first_non_empty = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        first_non_empty = Some(idx);
        break;
    }

    if let Some(idx) = first_non_empty {
        let mut line = lines[idx].trim();
        if let Some(stripped) = line.strip_prefix('\u{feff}') {
            line = stripped;
        }
        if line == "---" {
            saw_frontmatter = true;
        } else if lines.iter().skip(idx + 1).any(|line| line.trim() == "---") {
            errors.push(frontmatter_error(
                idx + 1,
                find_column(lines[idx], line),
                line.len().max(1),
                "Frontmatter must start at the beginning of the file.".to_string(),
                "Move the '---' delimiter to the top of the file.",
            ));
        }
    }

    if !saw_frontmatter {
        return errors;
    }

    let bounds = frontmatter_bounds(&lines);
    if bounds.is_none() {
        errors.push(frontmatter_error(
            1,
            1,
            3,
            "Frontmatter delimiter '---' is missing a closing delimiter.".to_string(),
            "Add a closing '---' to end the frontmatter section.",
        ));
        return errors;
    }
    let (_start, end) = bounds.unwrap();

    let mut in_block_comment = false;
    for (idx, line) in lines.iter().enumerate().skip(end + 1) {
        let clean = strip_php_tags_inline(line);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            continue;
        }
        if consume_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }
        if trimmed.starts_with("import ") {
            errors.push(frontmatter_error(
                idx + 1,
                find_column(line, "import"),
                trimmed.len().max(1),
                "Imports must appear in the frontmatter section.".to_string(),
                "Move the import statement above the closing '---'.",
            ));
        }
        if file_path.contains("php_modules/") && trimmed.starts_with("export ") {
            errors.push(frontmatter_error(
                idx + 1,
                find_column(line, "export"),
                trimmed.len().max(1),
                "Template files cannot declare exports.".to_string(),
                "Remove the export statement from the template section.",
            ));
        }
    }

    errors
}

pub fn validate_template_section(source: &str, file_path: &str) -> Vec<ValidationError> {
    let lines: Vec<&str> = source.lines().collect();
    let Some((start, end)) = frontmatter_bounds(&lines) else {
        return Vec::new();
    };
    let frontmatter_lines: Vec<&str> = lines
        .iter()
        .skip(start + 1)
        .take(end.saturating_sub(start + 1))
        .copied()
        .collect();
    let template_lines: Vec<&str> = lines.iter().skip(end + 1).copied().collect();
    if template_lines.iter().all(|line| line.trim().is_empty()) {
        return Vec::new();
    }

    // Template syntax validation runs through the PHPX parser. Frontmatter import/export
    // lines are handled by the module loader (not the parser), so leave them out here to
    // avoid recovery noise that can cascade into false JSX errors in template lines.
    let sanitized_frontmatter_lines: Vec<&str> = frontmatter_lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            let stripped_owned = strip_php_tags_inline(line);
            let stripped = stripped_owned.trim();
            if stripped.is_empty() {
                return Some(*line);
            }
            if stripped.starts_with("//")
                || stripped.starts_with("#")
                || stripped.starts_with("/*")
                || stripped.ends_with("*/")
            {
                return Some(*line);
            }
            if stripped.starts_with("import ")
                && parse_import_line(stripped, line, idx + 1, file_path).is_ok()
            {
                return None;
            }
            if stripped.starts_with("export ") {
                return None;
            }
            Some(*line)
        })
        .collect();
    let frontmatter = sanitized_frontmatter_lines.join("\n");
    let template = template_lines.join("\n");
    let prefix = "\n$__phpx_template = <__fragment__>\n";
    let suffix = "\n</__fragment__>;\n";
    let wrapped = format!("{frontmatter}{prefix}{template}{suffix}");

    let frontmatter_count = frontmatter.lines().count();
    let prefix_lines = prefix.lines().count();
    let template_wrapped_start_line = frontmatter_count + prefix_lines + 1;
    let template_start_line = end + 2;

    let arena = Bump::new();
    let lexer = Lexer::new(wrapped.as_bytes());
    let mut parser = Parser::new_with_mode(lexer, &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    let mut errors = Vec::new();

    for err in program.errors {
        let Some(info) = err.span.line_info(wrapped.as_bytes()) else {
            continue;
        };
        if info.line < template_wrapped_start_line {
            continue;
        }
        let template_line = info.line - template_wrapped_start_line + 1;
        let original_line = template_start_line + template_line - 1;
        let padding = std::cmp::min(info.line_text.len(), info.column.saturating_sub(1));
        let underline_length = std::cmp::max(
            1,
            std::cmp::min(err.span.len(), info.line_text.len().saturating_sub(padding)),
        );
        errors.push(ValidationError {
            kind: ErrorKind::JsxError,
            line: original_line,
            column: info.column,
            message: err.message.to_string(),
            help_text: "Fix JSX/template syntax in the template section.".to_string(),
            suggestion: None,
            underline_length,
            severity: Severity::Error,
        });
    }

    if let Err(type_errors) = check_program_with_path(&program, wrapped.as_bytes(), None) {
        for err in type_errors {
            let Some(info) = err.span.line_info(wrapped.as_bytes()) else {
                continue;
            };
            if info.line < template_wrapped_start_line {
                continue;
            }
            let template_line = info.line - template_wrapped_start_line + 1;
            let original_line = template_start_line + template_line - 1;
            let mut line = original_line;
            let mut column = info.column;
            let mut underline_length = {
                let padding = std::cmp::min(info.line_text.len(), info.column.saturating_sub(1));
                std::cmp::max(
                    1,
                    std::cmp::min(err.span.len(), info.line_text.len().saturating_sub(padding)),
                )
            };
            if let Some((tag_line, col, len)) = find_component_tag_location(
                &err.message,
                &lines,
                template_start_line,
                original_line,
            ) {
                line = tag_line;
                column = col;
                underline_length = len;
            }
            errors.push(ValidationError {
                kind: ErrorKind::TypeError,
                line,
                column,
                message: err.message,
                help_text: "Fix template component props or expression typing.".to_string(),
                suggestion: None,
                underline_length,
                severity: Severity::Error,
            });
        }
    }

    errors
}

fn component_name_from_message(message: &str) -> Option<String> {
    let marker = "component '";
    let start = message.find(marker)? + marker.len();
    let rest = &message[start..];
    let end_rel = rest.find('\'')?;
    let component = rest[..end_rel].trim();
    if component.is_empty() {
        return None;
    }
    Some(component.to_string())
}

fn find_component_tag_location(
    message: &str,
    lines: &[&str],
    template_start_line: usize,
    preferred_line: usize,
) -> Option<(usize, usize, usize)> {
    let component = component_name_from_message(message)?;
    let needle = format!("<{}", component);
    let mut best: Option<(usize, usize)> = None; // (line, col)
    for line_no in template_start_line..=lines.len() {
        let line_text = lines.get(line_no - 1).copied().unwrap_or("");
        if let Some(idx) = line_text.find(&needle) {
            let col = idx + 1; // 1-based
            match best {
                Some((best_line, _)) => {
                    let best_dist = best_line.abs_diff(preferred_line);
                    let this_dist = line_no.abs_diff(preferred_line);
                    if this_dist < best_dist {
                        best = Some((line_no, col));
                    }
                }
                None => {
                    best = Some((line_no, col));
                }
            }
        }
    }
    best.map(|(line_no, col)| (line_no, col, needle.len()))
}

struct JsxSyntaxValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for JsxSyntaxValidator<'_> {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        match expr {
            Expr::JsxElement {
                name, attributes, ..
            } => {
                self.check_tag_name(name);
                for attr in *attributes {
                    self.check_attribute(attr);
                }
            }
            Expr::JsxFragment { .. } => {}
            _ => {}
        }
        walk_expr(self, expr);
    }
}

impl JsxSyntaxValidator<'_> {
    fn check_tag_name(&mut self, name: &Name) {
        let Some(raw) = name_to_string(name, self.source) else {
            return;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            self.errors.push(jsx_error(
                name.span,
                self.source,
                "JSX tag name cannot be empty.".to_string(),
                "Provide a valid tag or component name.",
            ));
        }
    }

    fn check_attribute(&mut self, attr: &JsxAttribute) {
        let Some(name) = token_text(attr.name, self.source) else {
            return;
        };
        if !is_valid_attr_name(&name) {
            self.errors.push(jsx_error(
                attr.span,
                self.source,
                format!("Invalid JSX attribute name '{}'.", name),
                "Use a valid attribute identifier.",
            ));
            return;
        }
        if name.contains(':') && !is_island_directive_attr(&name) {
            self.errors.push(jsx_error(
                attr.span,
                self.source,
                format!("Invalid JSX attribute name '{}'.", name),
                "Only `client:*` or `server:defer` directive attributes may use ':'.",
            ));
        }
    }
}

struct JsxExpressionValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for JsxExpressionValidator<'_> {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        match expr {
            Expr::JsxElement {
                attributes,
                children,
                ..
            } => {
                for attr in *attributes {
                    if let Some(value) = attr.value {
                        if !is_jsx_expr(value) {
                            self.validate_expr(value);
                        }
                    }
                }
                for child in *children {
                    if let JsxChild::Expr(child_expr) = child {
                        if !is_jsx_expr(child_expr) {
                            self.validate_expr(child_expr);
                        }
                    }
                }
            }
            Expr::JsxFragment { children, .. } => {
                for child in *children {
                    if let JsxChild::Expr(child_expr) = child {
                        if !is_jsx_expr(child_expr) {
                            self.validate_expr(child_expr);
                        }
                    }
                }
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}

impl JsxExpressionValidator<'_> {
    fn validate_expr(&mut self, expr: ExprId<'_>) {
        let mut validator = JsxExprValidator {
            source: self.source,
            errors: Vec::new(),
        };
        validator.visit_expr(expr);
        self.errors.extend(validator.errors);
    }
}

struct JsxExprValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for JsxExprValidator<'_> {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        match expr {
            Expr::Binary {
                left, op, right, ..
            } => {
                if let Some(op_text) = comparison_operator(*op) {
                    if let Some(span) =
                        operator_span(self.source, left.span(), right.span(), op_text)
                    {
                        if !has_spacing_around_operator(self.source, span, op_text) {
                            self.errors.push(jsx_error(
                                span,
                                self.source,
                                format!("Add spaces around '{}' to avoid JSX ambiguity.", op_text),
                                "Use spaces around comparison operators inside JSX expressions.",
                            ));
                        }
                    }
                }
            }
            Expr::Assign { span, .. }
            | Expr::AssignRef { span, .. }
            | Expr::AssignOp { span, .. } => {
                self.errors.push(jsx_error(
                    *span,
                    self.source,
                    "Statements are not allowed inside JSX expressions.".to_string(),
                    "Use a value expression instead of an assignment.",
                ));
            }
            Expr::Yield { span, .. } => {
                self.errors.push(jsx_error(
                    *span,
                    self.source,
                    "Statements are not allowed inside JSX expressions.".to_string(),
                    "Remove the yield statement from JSX.",
                ));
            }
            Expr::Error { span } => {
                self.errors.push(jsx_error(
                    *span,
                    self.source,
                    "Invalid JSX expression.".to_string(),
                    "Fix the expression inside the JSX braces.",
                ));
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}

struct JsxComponentValidator<'a> {
    source: &'a str,
    known_components: HashSet<String>,
    async_components: HashSet<String>,
    suspense_depth: usize,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for JsxComponentValidator<'_> {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        if let Expr::JsxElement {
            name,
            attributes,
            children,
            ..
        } = *expr
        {
            let enters_suspense = self.validate_element(&name, attributes);
            if enters_suspense {
                self.suspense_depth += 1;
            }
            for attr in attributes.iter() {
                if let Some(value) = attr.value {
                    self.visit_expr(value);
                }
            }
            for child in children.iter() {
                if let JsxChild::Expr(inner) = *child {
                    self.visit_expr(inner);
                }
            }
            if enters_suspense {
                self.suspense_depth = self.suspense_depth.saturating_sub(1);
            }
            return;
        }
        walk_expr(self, expr);
    }
}

impl JsxComponentValidator<'_> {
    fn validate_element(&mut self, name: &Name, attributes: &[JsxAttribute]) -> bool {
        let Some(raw) = name_to_string(name, self.source) else {
            return false;
        };
        let trimmed = raw.trim_start_matches('\\');
        let last = trimmed.rsplit('\\').next().unwrap_or(trimmed);
        if last.is_empty() {
            return false;
        }

        let mut chars = last.chars();
        let is_component = chars
            .next()
            .map(|ch| ch.is_ascii_uppercase())
            .unwrap_or(false);
        let has_uppercase = last.chars().any(|ch| ch.is_ascii_uppercase());

        if !is_component && has_uppercase {
            self.errors.push(jsx_error(
                name.span,
                self.source,
                format!(
                    "JSX component '{}' must be capitalized (use <{} />).",
                    last,
                    capitalize_jsx_name(last)
                ),
                "Use a capitalized component name for components.",
            ));
            return false;
        }

        let directives: Vec<&JsxAttribute> = attributes
            .iter()
            .filter(|attr| {
                token_text(attr.name, self.source)
                    .map(|name| is_island_directive_attr(&name))
                    .unwrap_or(false)
            })
            .collect();

        if !is_component && !directives.is_empty() {
            self.errors.push(jsx_error(
                span_of_first_attr(directives[0]),
                self.source,
                "Island directives are only valid on components.".to_string(),
                "Use `client:*` or `server:defer` directives on a capitalized component tag.",
            ));
            return false;
        }

        if is_component && !self.known_components.contains(last) {
            self.errors.push(jsx_error(
                name.span,
                self.source,
                format!(
                    "Unknown component '{}'; import it or define function {}().",
                    last, last
                ),
                "Import the component or define a matching function.",
            ));
            return false;
        }

        if is_component {
            if directives.len() > 1 {
                self.errors.push(jsx_error(
                    name.span,
                    self.source,
                    "Only one hydration/render directive may be used per component.".to_string(),
                    "Use one of: `client:load`, `client:idle`, `client:visible`, `client:media`, `client:only`, `server:defer`.",
                ));
            }
            for attr in directives {
                self.validate_island_directive(attr);
            }
            let strict_async_suspense = std::env::var("PHPX_STRICT_ASYNC_SUSPENSE")
                .map(|value| {
                    let value = value.trim().to_ascii_lowercase();
                    value == "1" || value == "true" || value == "yes" || value == "on"
                })
                .unwrap_or(false);
            if self.async_components.contains(last)
                && self.suspense_depth == 0
                && last != "Suspense"
                && strict_async_suspense
            {
                self.errors.push(jsx_error(
                    name.span,
                    self.source,
                    format!("Async component '{}' must be wrapped in <Suspense>.", last),
                    "Wrap this component in `<Suspense fallback={...}>...</Suspense>`.",
                ));
            }
            self.validate_props(last, attributes, name.span);
        }
        is_component && last == "Suspense"
    }

    fn validate_island_directive(&mut self, attr: &JsxAttribute) {
        let Some(name) = token_text(attr.name, self.source) else {
            return;
        };
        if !(name == "client:media" || name == "clientMedia" || name == "client-media") {
            return;
        }
        let Some(value) = attr.value else {
            self.errors.push(jsx_error(
                attr.span,
                self.source,
                "client:media requires a media query value.".to_string(),
                "Provide a media query string, e.g. `client:media=\"(min-width: 768px)\"`.",
            ));
            return;
        };
        if !matches!(value, Expr::String { .. } | Expr::InterpolatedString { .. }) {
            self.errors.push(jsx_error(
                attr.span,
                self.source,
                "client:media requires a string value.".to_string(),
                "Provide a media query string literal.",
            ));
        }
    }

    fn validate_props(&mut self, component: &str, attributes: &[JsxAttribute], span: Span) {
        let mut attrs = HashSet::new();
        for attr in attributes {
            if let Some(name) = token_text(attr.name, self.source) {
                attrs.insert(name);
            }
        }

        match component {
            "Link" => {
                if !attrs.contains("to") {
                    self.errors.push(jsx_error(
                        span,
                        self.source,
                        "Link requires prop 'to'.".to_string(),
                        "Add a `to` attribute to Link.",
                    ));
                }
            }
            "ContextProvider" => {
                if !attrs.contains("ctx") {
                    self.errors.push(jsx_error(
                        span,
                        self.source,
                        "ContextProvider requires prop 'ctx'.".to_string(),
                        "Add a `ctx` attribute to ContextProvider.",
                    ));
                }
                if !attrs.contains("value") {
                    self.errors.push(jsx_error(
                        span,
                        self.source,
                        "ContextProvider requires prop 'value'.".to_string(),
                        "Add a `value` attribute to ContextProvider.",
                    ));
                }
            }
            "Suspense" => {
                if !attrs.contains("fallback") {
                    self.errors.push(jsx_error(
                        span,
                        self.source,
                        "Suspense requires prop 'fallback'.".to_string(),
                        "Add a `fallback` prop, e.g. `<Suspense fallback={<div>Loading...</div>}>`.",
                    ));
                }
            }
            _ => {}
        }
    }
}

fn collect_function_names(program: &Program, source: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in program.statements {
        if let Stmt::Function { name, .. } = stmt {
            if let Some(text) = token_text(name, source) {
                names.insert(text);
            }
        }
    }
    names
}

fn collect_async_function_names(program: &Program, source: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in program.statements {
        if let Stmt::Function { name, is_async, .. } = stmt {
            if *is_async {
                if let Some(text) = token_text(name, source) {
                    names.insert(text);
                }
            }
        }
    }
    names
}

fn collect_imported_names(source: &str) -> HashSet<String> {
    let lines: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_bounds(&lines);
    let scan_end = bounds.map(|(_, end)| end).unwrap_or(lines.len());
    let mut in_block_comment = false;
    let mut imported = HashSet::new();
    for (idx, line) in lines.iter().enumerate().take(scan_end) {
        if let Some((start, end)) = bounds {
            if idx == start || idx == end {
                continue;
            }
        }
        let clean = strip_php_tags_inline(line);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            continue;
        }
        if consume_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }
        if trimmed.starts_with("import ") {
            if let Ok(specs) = parse_import_line(trimmed, line, idx + 1, "<source>") {
                for spec in specs {
                    imported.insert(spec.local);
                }
            }
        }
    }
    imported
}

fn jsx_error(span: Span, source: &str, message: String, help_text: &str) -> ValidationError {
    let (line, column, underline_length) = span_location(span, source);
    ValidationError {
        kind: ErrorKind::JsxError,
        line,
        column,
        message,
        help_text: help_text.to_string(),
        suggestion: None,
        underline_length,
        severity: Severity::Error,
    }
}

fn frontmatter_error(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
) -> ValidationError {
    ValidationError {
        kind: ErrorKind::JsxError,
        line,
        column,
        message,
        help_text: help_text.to_string(),
        suggestion: None,
        underline_length: underline_length.max(1),
        severity: Severity::Error,
    }
}

fn span_location(span: Span, source: &str) -> (usize, usize, usize) {
    if let Some(info) = span.line_info(source.as_bytes()) {
        let padding = std::cmp::min(info.line_text.len(), info.column.saturating_sub(1));
        let highlight_len = std::cmp::max(
            1,
            std::cmp::min(span.len(), info.line_text.len().saturating_sub(padding)),
        );
        (info.line, info.column, highlight_len)
    } else {
        (1, 1, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::validate_components;
    use bumpalo::Bump;
    use php_rs::parser::lexer::Lexer;
    use php_rs::parser::parser::{Parser, ParserMode};

    #[test]
    fn async_component_requires_suspense_wrapper() {
        // This check is gated by PHPX_STRICT_ASYNC_SUSPENSE env var
        unsafe {
            std::env::set_var("PHPX_STRICT_ASYNC_SUSPENSE", "1");
        }
        let source = r#"
async function Card($props: Object<{ label: string }>): Promise<VNode> {
    return <div>{$props.label}</div>
}
<div><Card label="x" /></div>
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let errors = validate_components(&program, source);
        unsafe {
            std::env::remove_var("PHPX_STRICT_ASYNC_SUSPENSE");
        }
        assert!(
            errors
                .iter()
                .any(|err| err.message.contains("must be wrapped in <Suspense>")),
            "expected suspense wrapper error, got: {:?}",
            errors
        );
    }

    #[test]
    fn suspense_with_fallback_allows_async_component() {
        let source = r#"
async function Card($props: Object<{ label: string }>): Promise<VNode> {
    return <div>{$props.label}</div>
}
<Suspense fallback={<div>Loading</div>}>
  <Card label="x" />
</Suspense>
"#;
        let arena = Bump::new();
        let mut parser =
            Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
        let program = parser.parse_program();
        let errors = validate_components(&program, source);
        assert!(
            !errors
                .iter()
                .any(|err| err.message.contains("must be wrapped in <Suspense>")),
            "unexpected suspense wrapper error: {:?}",
            errors
        );
    }
}

fn comparison_operator(op: BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Lt => Some("<"),
        BinaryOp::LtEq => Some("<="),
        BinaryOp::Gt => Some(">"),
        BinaryOp::GtEq => Some(">="),
        BinaryOp::Spaceship => Some("<=>"),
        _ => None,
    }
}

fn operator_span(source: &str, left: Span, right: Span, op: &str) -> Option<Span> {
    let start = left.end.min(source.len());
    let end = right.start.min(source.len());
    if end <= start {
        return None;
    }
    let slice = source.as_bytes();
    let between = &slice[start..end];
    let op_bytes = op.as_bytes();
    let pos = between
        .windows(op_bytes.len())
        .position(|window| window == op_bytes)?;
    let op_start = start + pos;
    Some(Span::new(op_start, op_start + op_bytes.len()))
}

fn has_spacing_around_operator(source: &str, span: Span, op: &str) -> bool {
    if span.start >= source.len() || span.end > source.len() {
        return true;
    }
    let bytes = source.as_bytes();
    let op_start = span.start;
    let op_end = span.end;

    if op_end <= op_start {
        return true;
    }

    let before = if op_start == 0 {
        None
    } else {
        bytes.get(op_start - 1)
    };
    let after = bytes.get(op_end);

    let before_ok = before.map(|b| b.is_ascii_whitespace()).unwrap_or(false);
    let after_ok = after.map(|b| b.is_ascii_whitespace()).unwrap_or(false);

    if !before_ok || !after_ok {
        return false;
    }

    // Guard against accidental substring matches in uncommon cases.
    &source[op_start..op_end] == op
}

fn name_to_string(name: &Name, source: &str) -> Option<String> {
    std::str::from_utf8(name.span.as_str(source.as_bytes()))
        .ok()
        .map(|text| text.to_string())
}

fn token_text(token: &php_rs::parser::lexer::token::Token, source: &str) -> Option<String> {
    std::str::from_utf8(token.text(source.as_bytes()))
        .ok()
        .map(|text| text.to_string())
}

fn is_valid_attr_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric() || ch == '-' || ch == ':')
}

fn is_island_directive_attr(name: &str) -> bool {
    matches!(
        name,
        "client:load"
            | "clientLoad"
            | "client-load"
            | "client:idle"
            | "clientIdle"
            | "client-idle"
            | "client:visible"
            | "clientVisible"
            | "client-visible"
            | "client:media"
            | "clientMedia"
            | "client-media"
            | "client:only"
            | "clientOnly"
            | "client-only"
            | "server:defer"
            | "serverDefer"
            | "server-defer"
    )
}

fn span_of_first_attr(attr: &JsxAttribute) -> Span {
    attr.span
}

fn capitalize_jsx_name(name: &str) -> String {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.push(first.to_ascii_uppercase());
    out.push_str(chars.as_str());
    out
}

fn is_jsx_expr(expr: ExprId<'_>) -> bool {
    matches!(expr, Expr::JsxElement { .. } | Expr::JsxFragment { .. })
}
