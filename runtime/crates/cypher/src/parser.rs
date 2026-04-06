use nom::{
    IResult,
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while, take_while1},
    character::complete::{char, multispace0, multispace1},
    combinator::{opt, recognize},
    multi::separated_list1,
    sequence::{delimited, pair, preceded, terminated, tuple},
};

use crate::ast::*;

/// Parse result with rich errors.
#[derive(Debug)]
pub struct ParseResult {
    pub statement: Option<CypherStatement>,
    pub errors: Vec<CypherError>,
    pub params: Vec<ParamRef>,
}

#[derive(Debug, Clone)]
pub struct CypherError {
    pub span: Span,
    pub message: String,
    pub help: String,
}

#[derive(Debug, Clone)]
pub struct ParamRef {
    pub name: String,
    pub span: Span,
}

/// Main entry point: parse a Cypher query string.
pub fn parse_cypher(input: &str) -> ParseResult {
    let mut params = Vec::new();
    match parse_statement(input, &mut params) {
        Ok((remaining, statement)) => {
            let trimmed = remaining.trim();
            let mut errors = Vec::new();
            if !trimmed.is_empty() {
                let offset = input.len() - remaining.len();
                errors.push(CypherError {
                    span: Span::new(offset, input.len()),
                    message: "Unexpected content after query".to_string(),
                    help: "A cql statement should contain a single complete Cypher query."
                        .to_string(),
                });
            }
            ParseResult {
                statement: Some(statement),
                errors,
                params,
            }
        }
        Err(e) => {
            let offset = match &e {
                nom::Err::Error(e) | nom::Err::Failure(e) => {
                    input.len() - e.input.len()
                }
                _ => 0,
            };
            ParseResult {
                statement: None,
                errors: vec![CypherError {
                    span: Span::new(offset, (offset + 10).min(input.len())),
                    message: "Failed to parse Cypher query".to_string(),
                    help: "Check the syntax near the highlighted position.".to_string(),
                }],
                params,
            }
        }
    }
}

// ─── Span tracking ─────────────────────────────────────────────

fn offset(input: &str, original: &str) -> usize {
    input.as_ptr() as usize - original.as_ptr() as usize
}

fn span_from(start: &str, end: &str, original: &str) -> Span {
    Span::new(offset(start, original), offset(end, original))
}

// ─── Whitespace / utility ──────────────────────────────────────

fn ws(input: &str) -> IResult<&str, &str> {
    multispace0(input)
}

fn keyword<'a>(kw: &'static str) -> impl FnMut(&'a str) -> IResult<&'a str, &'a str> {
    move |input: &'a str| {
        let (rest, matched) = tag_no_case(kw)(input)?;
        // Ensure keyword is not part of a longer identifier
        if rest
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )))
        } else {
            Ok((rest, matched))
        }
    }
}

fn comma_sep(input: &str) -> IResult<&str, &str> {
    delimited(ws, tag(","), ws)(input)
}

fn try_tag<'a>(t: &'static str, input: &'a str) -> IResult<&'a str, &'a str> {
    tag(t)(input)
}

fn try_char(c: char, input: &str) -> IResult<&str, char> {
    char(c)(input)
}

// ─── Identifiers & literals ────────────────────────────────────

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_cont(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Reserved words that cannot be used as identifiers without backtick-quoting.
fn is_reserved(s: &str) -> bool {
    matches!(
        s.to_uppercase().as_str(),
        "MATCH" | "OPTIONAL" | "WHERE" | "RETURN" | "CREATE" | "DELETE" | "DETACH"
        | "SET" | "REMOVE" | "MERGE" | "WITH" | "UNWIND" | "UNION" | "ORDER"
        | "BY" | "SKIP" | "LIMIT" | "AS" | "AND" | "OR" | "XOR" | "NOT" | "IN"
        | "IS" | "NULL" | "TRUE" | "FALSE" | "DISTINCT" | "ASC" | "DESC"
        | "ON" | "CASE" | "WHEN" | "THEN" | "ELSE" | "END" | "STARTS" | "ENDS"
        | "CONTAINS" | "EXISTS" | "ALL" | "CALL" | "YIELD"
    )
}

fn identifier(input: &str) -> IResult<&str, String> {
    // Backtick-quoted identifier
    if input.starts_with('`') {
        let (rest, _) = char('`')(input)?;
        let (rest, name) = take_while(|c| c != '`')(rest)?;
        let (rest, _) = char('`')(rest)?;
        return Ok((rest, name.to_string()));
    }
    let (rest, name) = recognize(pair(
        take_while1(|c: char| is_ident_start(c)),
        take_while(|c: char| is_ident_cont(c)),
    ))(input)?;
    if is_reserved(name) {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }
    Ok((rest, name.to_string()))
}

/// Identifier that allows reserved words (used for label names, relationship types).
fn symbolic_name(input: &str) -> IResult<&str, String> {
    if input.starts_with('`') {
        let (rest, _) = char('`')(input)?;
        let (rest, name) = take_while(|c| c != '`')(rest)?;
        let (rest, _) = char('`')(rest)?;
        return Ok((rest, name.to_string()));
    }
    let (rest, name) = recognize(pair(
        take_while1(|c: char| is_ident_start(c)),
        take_while(|c: char| is_ident_cont(c)),
    ))(input)?;
    Ok((rest, name.to_string()))
}

fn parse_param<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (rest, _) = char('$')(input)?;
    let (rest, name) = recognize(pair(
        take_while1(|c: char| is_ident_start(c)),
        take_while(|c: char| is_ident_cont(c)),
    ))(rest)?;
    let sp = span_from(start, rest, original);
    params.push(ParamRef {
        name: name.to_string(),
        span: sp,
    });
    Ok((rest, Expr::Param {
        name: name.to_string(),
        span: sp,
    }))
}

fn integer_literal(input: &str) -> IResult<&str, (i64, Span)> {
    let start = input;
    let (rest, neg) = opt(char('-'))(input)?;
    let (rest, digits) = take_while1(|c: char| c.is_ascii_digit())(rest)?;
    // Make sure it's not a float
    if rest.starts_with('.') && rest.get(1..2).is_some_and(|c| c.chars().next().unwrap().is_ascii_digit()) {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Float,
        )));
    }
    let s = if neg.is_some() {
        format!("-{}", digits)
    } else {
        digits.to_string()
    };
    let val: i64 = s.parse().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit))
    })?;
    let original_start = start;
    // We need the original pointer to compute spans, but we don't have it here.
    // We'll compute spans at a higher level. For now use dummy spans.
    Ok((rest, (val, Span::new(0, 0))))
}

fn float_literal(input: &str) -> IResult<&str, (f64, Span)> {
    let (rest, num) = recognize(tuple((
        opt(char('-')),
        take_while1(|c: char| c.is_ascii_digit()),
        char('.'),
        take_while1(|c: char| c.is_ascii_digit()),
    )))(input)?;
    let val: f64 = num.parse().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Float))
    })?;
    Ok((rest, (val, Span::new(0, 0))))
}

fn string_literal(input: &str) -> IResult<&str, String> {
    let quote = if input.starts_with('\'') {
        '\''
    } else if input.starts_with('"') {
        '"'
    } else {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Char,
        )));
    };

    let (rest, _) = char(quote)(input)?;
    let mut result = String::new();
    let mut chars = rest.char_indices();
    loop {
        match chars.next() {
            Some((_, '\\')) => {
                if let Some((_, c)) = chars.next() {
                    match c {
                        'n' => result.push('\n'),
                        't' => result.push('\t'),
                        '\\' => result.push('\\'),
                        c if c == quote => result.push(quote),
                        _ => {
                            result.push('\\');
                            result.push(c);
                        }
                    }
                }
            }
            Some((i, c)) if c == quote => {
                return Ok((&rest[i + 1..], result));
            }
            Some((_, c)) => result.push(c),
            None => {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Char,
                )));
            }
        }
    }
}

// ─── Statement / Query ─────────────────────────────────────────

fn parse_statement<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
) -> IResult<&'a str, CypherStatement> {
    let start = input;
    let (rest, query) = parse_query(input, params)?;
    let sp = span_from(start, rest, start);
    Ok((rest, CypherStatement { query, span: sp }))
}

fn parse_query<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
) -> IResult<&'a str, Query> {
    let (mut rest, first) = parse_single_query(input, params)?;

    let mut query = Query::Single(first);
    loop {
        let (r, _) = ws(rest)?;
        if let Ok((r2, _)) = keyword("UNION")(r) {
            let (r3, _) = ws(r2)?;
            let all = if let Ok((r4, _)) = keyword("ALL")(r3) {
                rest = r4;
                true
            } else {
                rest = r3;
                false
            };
            let (r5, _) = ws(rest)?;
            let (r6, right) = parse_single_query(r5, params)?;
            query = Query::Union {
                left: Box::new(query),
                all,
                right,
            };
            rest = r6;
        } else {
            break;
        }
    }
    Ok((rest, query))
}

fn parse_single_query<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
) -> IResult<&'a str, SingleQuery> {
    let start = input;
    let (rest, _) = ws(input)?;
    let mut clauses = Vec::new();
    let mut current = rest;

    loop {
        let (r, _) = ws(current)?;
        if r.is_empty() {
            current = r;
            break;
        }

        // Try each clause parser
        if let Ok((r2, clause)) = parse_clause(r, params, start) {
            clauses.push(clause);
            current = r2;
        } else {
            break;
        }
    }

    if clauses.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Many1,
        )));
    }

    let sp = span_from(start, current, start);
    Ok((current, SingleQuery { clauses, span: sp }))
}

fn parse_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    // Try each clause parser sequentially to avoid borrow conflicts with alt()
    if let Ok(r) = parse_match_clause(input, params, original) { return Ok(r); }
    if let Ok(r) = parse_create_clause(input, params, original) { return Ok(r); }
    if let Ok(r) = parse_merge_clause(input, params, original) { return Ok(r); }
    if let Ok(r) = parse_return_clause(input, params, original) { return Ok(r); }
    if let Ok(r) = parse_with_clause(input, params, original) { return Ok(r); }
    if let Ok(r) = parse_unwind_clause(input, params, original) { return Ok(r); }
    if let Ok(r) = parse_delete_clause(input, params, original) { return Ok(r); }
    if let Ok(r) = parse_set_clause(input, params, original) { return Ok(r); }
    if let Ok(r) = parse_remove_clause(input, params, original) { return Ok(r); }
    Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Alt)))
}

// ─── MATCH ─────────────────────────────────────────────────────

fn parse_match_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, optional) = opt(terminated(keyword("OPTIONAL"), multispace1))(input)?;
    let (rest, _) = keyword("MATCH")(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, pattern) = parse_pattern(rest, params, original)?;
    let (rest, _) = ws(rest)?;
    let (rest, where_clause) = opt(|i| parse_where(i, params, original))(rest)?;
    let sp = span_from(start, rest, original);
    Ok((
        rest,
        Clause::Match {
            optional: optional.is_some(),
            pattern,
            where_clause,
            span: sp,
        },
    ))
}

fn parse_where<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let (rest, _) = keyword("WHERE")(input)?;
    let (rest, _) = ws(rest)?;
    parse_expr(rest, params, original)
}

// ─── CREATE ────────────────────────────────────────────────────

fn parse_create_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, _) = keyword("CREATE")(input)?;
    let (rest, _) = ws(rest)?;
    let (rest, pattern) = parse_pattern(rest, params, original)?;
    let sp = span_from(start, rest, original);
    Ok((rest, Clause::Create { pattern, span: sp }))
}

// ─── MERGE ─────────────────────────────────────────────────────

fn parse_merge_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, _) = keyword("MERGE")(input)?;
    let (rest, _) = ws(rest)?;
    let (rest, pattern) = parse_pattern_part(rest, params, original)?;
    let (rest, _) = ws(rest)?;

    let mut on_create = None;
    let mut on_match = None;
    let mut current = rest;

    loop {
        let (r, _) = ws(current)?;
        if let Ok((r2, _)) = pair(keyword("ON"), preceded(multispace1, keyword("CREATE")))(r) {
            let (r3, _) = preceded(ws, keyword("SET"))(r2)?;
            let (r4, _) = ws(r3)?;
            let (r5, items) = parse_set_items(r4, params, original)?;
            on_create = Some(items);
            current = r5;
        } else if let Ok((r2, _)) = pair(keyword("ON"), preceded(multispace1, keyword("MATCH")))(r)
        {
            let (r3, _) = preceded(ws, keyword("SET"))(r2)?;
            let (r4, _) = ws(r3)?;
            let (r5, items) = parse_set_items(r4, params, original)?;
            on_match = Some(items);
            current = r5;
        } else {
            break;
        }
    }

    let sp = span_from(start, current, original);
    Ok((
        current,
        Clause::Merge {
            pattern,
            on_create,
            on_match,
            span: sp,
        },
    ))
}

// ─── RETURN ────────────────────────────────────────────────────

fn parse_return_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, _) = keyword("RETURN")(input)?;
    let (rest, _) = ws(rest)?;
    let (rest, distinct) = opt(terminated(keyword("DISTINCT"), multispace1))(rest)?;
    let (rest, items) = parse_return_items(rest, params, original)?;
    let (rest, _) = ws(rest)?;
    let (rest, order_by) = opt(|i| parse_order_by(i, params, original))(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, skip) = opt(|i| parse_skip(i, params, original))(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, limit) = opt(|i| parse_limit(i, params, original))(rest)?;
    let sp = span_from(start, rest, original);
    Ok((
        rest,
        Clause::Return {
            distinct: distinct.is_some(),
            items,
            order_by,
            skip,
            limit,
            span: sp,
        },
    ))
}

// ─── WITH ──────────────────────────────────────────────────────

fn parse_with_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, _) = keyword("WITH")(input)?;
    let (rest, _) = ws(rest)?;
    let (rest, distinct) = opt(terminated(keyword("DISTINCT"), multispace1))(rest)?;
    let (rest, items) = parse_return_items(rest, params, original)?;
    let (rest, _) = ws(rest)?;
    let (rest, order_by) = opt(|i| parse_order_by(i, params, original))(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, skip) = opt(|i| parse_skip(i, params, original))(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, limit) = opt(|i| parse_limit(i, params, original))(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, where_clause) = opt(|i| parse_where(i, params, original))(rest)?;
    let sp = span_from(start, rest, original);
    Ok((
        rest,
        Clause::With {
            distinct: distinct.is_some(),
            items,
            where_clause,
            order_by,
            skip,
            limit,
            span: sp,
        },
    ))
}

// ─── UNWIND ────────────────────────────────────────────────────

fn parse_unwind_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, _) = keyword("UNWIND")(input)?;
    let (rest, _) = ws(rest)?;
    let (rest, expr) = parse_expr(rest, params, original)?;
    let (rest, _) = ws(rest)?;
    let (rest, _) = keyword("AS")(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, alias) = symbolic_name(rest)?;
    let sp = span_from(start, rest, original);
    Ok((
        rest,
        Clause::Unwind {
            expr,
            alias,
            span: sp,
        },
    ))
}

// ─── DELETE ────────────────────────────────────────────────────

fn parse_delete_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, detach) = opt(terminated(keyword("DETACH"), multispace1))(input)?;
    let (rest, _) = keyword("DELETE")(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, first) = parse_expr(rest, params, original)?;
    let mut exprs = vec![first];
    let mut current = rest;
    while let Ok((r, _)) = comma_sep(current) {
        let (r2, expr) = parse_expr(r, params, original)?;
        exprs.push(expr);
        current = r2;
    }
    let sp = span_from(start, current, original);
    Ok((
        current,
        Clause::Delete {
            detach: detach.is_some(),
            exprs,
            span: sp,
        },
    ))
}

// ─── SET ───────────────────────────────────────────────────────

fn parse_set_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, _) = keyword("SET")(input)?;
    let (rest, _) = ws(rest)?;
    let (rest, items) = parse_set_items(rest, params, original)?;
    let sp = span_from(start, rest, original);
    Ok((rest, Clause::Set { items, span: sp }))
}

fn parse_set_items<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Vec<SetItem>> {
    let (rest, first) = parse_set_item(input, params, original)?;
    let mut items = vec![first];
    let mut current = rest;
    while let Ok((r, _)) = comma_sep(current) {
        let (r2, item) = parse_set_item(r, params, original)?;
        items.push(item);
        current = r2;
    }
    Ok((current, items))
}

fn parse_set_item<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, SetItem> {
    // Parse target as a postfix expression only (no binary ops) so we don't consume the `=`
    let (rest, target) = parse_postfix_expr(input, params, original)?;
    let (rest, _) = ws(rest)?;

    // Check for += (merge properties)
    if let Ok((rest2, _)) = try_tag("+=", rest) {
        let (rest3, _) = ws(rest2)?;
        let (rest4, val) = parse_expr(rest3, params, original)?;
        if let Expr::Ident { name, .. } = &target {
            return Ok((
                rest4,
                SetItem::AllProperties {
                    variable: name.clone(),
                    value: val,
                    merge: true,
                },
            ));
        }
    }

    let (rest, _) = char('=')(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, val) = parse_expr(rest, params, original)?;
    Ok((
        rest,
        SetItem::Property {
            target,
            value: val,
        },
    ))
}

// ─── REMOVE ────────────────────────────────────────────────────

fn parse_remove_clause<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Clause> {
    let start = input;
    let (rest, _) = keyword("REMOVE")(input)?;
    let (rest, _) = ws(rest)?;

    let (rest, first) = parse_remove_item(rest, params, original)?;
    let mut items = vec![first];
    let mut current = rest;
    while let Ok((r, _)) = comma_sep(current) {
        let (r2, item) = parse_remove_item(r, params, original)?;
        items.push(item);
        current = r2;
    }
    let sp = span_from(start, current, original);
    Ok((current, Clause::Remove { items, span: sp }))
}

fn parse_remove_item<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, RemoveItem> {
    // Try label removal: var:Label
    if let Ok((rest, name)) = symbolic_name(input) {
        if let Ok((rest2, _)) = try_char(':', rest) {
            let (rest3, labels) = separated_list1(char(':'), symbolic_name)(rest2)?;
            return Ok((
                rest3,
                RemoveItem::Label {
                    variable: name,
                    labels,
                },
            ));
        }
    }
    // Property removal
    let (rest, expr) = parse_expr(input, params, original)?;
    Ok((rest, RemoveItem::Property(expr)))
}

// ─── Return items / ORDER BY / SKIP / LIMIT ────────────────────

fn parse_return_items<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Vec<ReturnItem>> {
    // Handle RETURN *
    if let Ok((rest, _)) = try_char('*', input) {
        return Ok((
            rest,
            vec![ReturnItem {
                expr: Expr::Ident {
                    name: "*".to_string(),
                    span: span_from(input, rest, original),
                },
                alias: None,
                span: span_from(input, rest, original),
            }],
        ));
    }

    let (rest, first) = parse_return_item(input, params, original)?;
    let mut items = vec![first];
    let mut current = rest;
    while let Ok((r, _)) = comma_sep(current) {
        let (r2, item) = parse_return_item(r, params, original)?;
        items.push(item);
        current = r2;
    }
    Ok((current, items))
}

fn parse_return_item<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, ReturnItem> {
    let start = input;
    let (rest, expr) = parse_expr(input, params, original)?;
    let (rest, _) = ws(rest)?;
    let (rest, alias) = opt(preceded(
        terminated(keyword("AS"), multispace1),
        symbolic_name,
    ))(rest)?;
    let sp = span_from(start, rest, original);
    Ok((rest, ReturnItem { expr, alias, span: sp }))
}

fn parse_order_by<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Vec<OrderItem>> {
    let (rest, _) = keyword("ORDER")(input)?;
    let (rest, _) = multispace1(rest)?;
    let (rest, _) = keyword("BY")(rest)?;
    let (rest, _) = ws(rest)?;

    let (rest, first) = parse_order_item(rest, params, original)?;
    let mut items = vec![first];
    let mut current = rest;
    while let Ok((r, _)) = comma_sep(current) {
        let (r2, item) = parse_order_item(r, params, original)?;
        items.push(item);
        current = r2;
    }
    Ok((current, items))
}

fn parse_order_item<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, OrderItem> {
    let (rest, expr) = parse_expr(input, params, original)?;
    let (rest, _) = ws(rest)?;
    let (rest, dir) = opt(alt((keyword("ASC"), keyword("DESC"))))(rest)?;
    let ascending = !matches!(dir, Some(d) if d.eq_ignore_ascii_case("DESC"));
    Ok((rest, OrderItem { expr, ascending }))
}

fn parse_skip<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let (rest, _) = keyword("SKIP")(input)?;
    let (rest, _) = ws(rest)?;
    parse_expr(rest, params, original)
}

fn parse_limit<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let (rest, _) = keyword("LIMIT")(input)?;
    let (rest, _) = ws(rest)?;
    parse_expr(rest, params, original)
}

// ─── Pattern parsing ───────────────────────────────────────────

fn parse_pattern<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Vec<PatternPart>> {
    let (rest, first) = parse_pattern_part(input, params, original)?;
    let mut parts = vec![first];
    let mut current = rest;
    while let Ok((r, _)) = comma_sep(current) {
        let (r2, part) = parse_pattern_part(r, params, original)?;
        parts.push(part);
        current = r2;
    }
    Ok((current, parts))
}

fn parse_pattern_part<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, PatternPart> {
    let start = input;

    // Optional named pattern: var = (pattern)
    let (rest, variable) = opt(terminated(
        identifier,
        delimited(ws, char('='), ws),
    ))(input)?;

    let rest = if variable.is_some() { rest } else { input };

    // Parse node-[rel]->node chain
    let (rest, node) = parse_node_pattern(rest, params, original)?;
    let mut path = vec![node];
    let mut current = rest;

    loop {
        let (r, _) = ws(current)?;
        if let Ok((r2, (rel, node))) = parse_rel_and_node(r, params, original) {
            path.push(rel);
            path.push(node);
            current = r2;
        } else {
            break;
        }
    }

    let sp = span_from(start, current, original);
    Ok((
        current,
        PatternPart {
            variable,
            path,
            span: sp,
        },
    ))
}

fn parse_node_pattern<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, PatternElement> {
    let start = input;
    let (rest, _) = char('(')(input)?;
    let (rest, _) = ws(rest)?;

    // Optional variable
    let (rest, variable) = opt(identifier)(rest)?;
    let (rest, _) = ws(rest)?;

    // Optional labels :Label:Label2
    let mut labels = Vec::new();
    let mut current = rest;
    while let Ok((r, _)) = try_char(':', current) {
        let (r2, label) = symbolic_name(r)?;
        labels.push(label);
        current = r2;
    }
    let (current, _) = ws(current)?;

    // Optional properties { key: value }
    let (current, properties) = opt(|i| parse_map_literal(i, params, original))(current)?;
    let (current, _) = ws(current)?;

    let (rest, _) = char(')')(current)?;
    let sp = span_from(start, rest, original);
    Ok((
        rest,
        PatternElement::Node {
            variable,
            labels,
            properties,
            span: sp,
        },
    ))
}

fn parse_rel_and_node<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, (PatternElement, PatternElement)> {
    let (rest, rel) = parse_relationship_pattern(input, params, original)?;
    let (rest, _) = ws(rest)?;
    let (rest, node) = parse_node_pattern(rest, params, original)?;
    Ok((rest, (rel, node)))
}

fn parse_relationship_pattern<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, PatternElement> {
    let start = input;

    // Determine direction from prefix
    let (rest, left_arrow) = opt(tag("<-"))(input)?;
    let rest = if left_arrow.is_some() {
        rest
    } else {
        let (r, _) = char('-')(input)?;
        r
    };

    // Optional bracket details [var:TYPE*1..3]
    let (rest, details) = opt(|i| parse_rel_detail(i, params, original))(rest)?;

    // If no bracket, just consume the dash/arrow
    let rest = if details.is_none() {
        rest
    } else {
        rest
    };

    // Determine direction from suffix
    let (rest, right_arrow) = opt(tag("->"))(rest)?;
    let rest = if right_arrow.is_some() {
        rest
    } else {
        let (r, _) = char('-')(rest)?;
        r
    };

    let direction = match (left_arrow.is_some(), right_arrow.is_some()) {
        (false, true) => Direction::Right,
        (true, false) => Direction::Left,
        (true, true) => Direction::Both,
        (false, false) => Direction::Undirected,
    };

    let (variable, rel_types, properties, length) = details
        .map(|(v, t, p, l)| (v, t, p, l))
        .unwrap_or_default();

    let sp = span_from(start, rest, original);
    Ok((
        rest,
        PatternElement::Relationship {
            variable,
            rel_types,
            direction,
            properties,
            length,
            span: sp,
        },
    ))
}

fn parse_rel_detail<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<
    &'a str,
    (
        Option<String>,
        Vec<String>,
        Option<Expr>,
        Option<RangeLength>,
    ),
> {
    let (rest, _) = char('[')(input)?;
    let (rest, _) = ws(rest)?;

    // Optional variable
    let (rest, variable) = opt(identifier)(rest)?;
    let (rest, _) = ws(rest)?;

    // Optional relationship types :TYPE|TYPE2
    let mut rel_types = Vec::new();
    let mut current = rest;
    if let Ok((r, _)) = try_char(':', current) {
        let (r2, first) = symbolic_name(r)?;
        rel_types.push(first);
        current = r2;
        while let Ok((r3, _)) = try_char('|', current) {
            // Allow optional : after |
            let r3 = if let Ok((r4, _)) = try_char(':', r3) {
                r4
            } else {
                r3
            };
            let (r4, t) = symbolic_name(r3)?;
            rel_types.push(t);
            current = r4;
        }
    }
    let (current, _) = ws(current)?;

    // Optional variable-length *min..max
    let (current, length) = opt(parse_range_length)(current)?;
    let (current, _) = ws(current)?;

    // Optional properties
    let (current, properties) = opt(|i| parse_map_literal(i, params, original))(current)?;
    let (current, _) = ws(current)?;

    let (rest, _) = char(']')(current)?;
    Ok((rest, (variable, rel_types, properties, length)))
}

fn parse_range_length(input: &str) -> IResult<&str, RangeLength> {
    let (rest, _) = char('*')(input)?;
    let (rest, _) = ws(rest)?;

    let (rest, min) = opt(take_while1(|c: char| c.is_ascii_digit()))(rest)?;
    let (rest, _) = ws(rest)?;

    let (rest, has_dots) = opt(tag(".."))(rest)?;
    let (rest, _) = ws(rest)?;

    let (rest, max) = if has_dots.is_some() {
        opt(take_while1(|c: char| c.is_ascii_digit()))(rest)?
    } else {
        (rest, None)
    };

    Ok((
        rest,
        RangeLength {
            min: min.and_then(|s: &str| s.parse().ok()),
            max: max.and_then(|s: &str| s.parse().ok()),
        },
    ))
}

// ─── Expression parsing (Pratt-style precedence) ───────────────

fn parse_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    parse_or_expr(input, params, original)
}

fn parse_or_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (mut rest, mut left) = parse_xor_expr(input, params, original)?;
    loop {
        let (r, _) = ws(rest)?;
        if let Ok((r2, _)) = keyword("OR")(r) {
            let (r3, _) = ws(r2)?;
            let (r4, right) = parse_xor_expr(r3, params, original)?;
            let sp = span_from(start, r4, original);
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::Or,
                right: Box::new(right),
                span: sp,
            };
            rest = r4;
        } else {
            break;
        }
    }
    Ok((rest, left))
}

fn parse_xor_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (mut rest, mut left) = parse_and_expr(input, params, original)?;
    loop {
        let (r, _) = ws(rest)?;
        if let Ok((r2, _)) = keyword("XOR")(r) {
            let (r3, _) = ws(r2)?;
            let (r4, right) = parse_and_expr(r3, params, original)?;
            let sp = span_from(start, r4, original);
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::Xor,
                right: Box::new(right),
                span: sp,
            };
            rest = r4;
        } else {
            break;
        }
    }
    Ok((rest, left))
}

fn parse_and_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (mut rest, mut left) = parse_not_expr(input, params, original)?;
    loop {
        let (r, _) = ws(rest)?;
        if let Ok((r2, _)) = keyword("AND")(r) {
            let (r3, _) = ws(r2)?;
            let (r4, right) = parse_not_expr(r3, params, original)?;
            let sp = span_from(start, r4, original);
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::And,
                right: Box::new(right),
                span: sp,
            };
            rest = r4;
        } else {
            break;
        }
    }
    Ok((rest, left))
}

fn parse_not_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    if let Ok((rest, _)) = keyword("NOT")(input) {
        let (rest, _) = ws(rest)?;
        let (rest, expr) = parse_not_expr(rest, params, original)?;
        let sp = span_from(start, rest, original);
        Ok((
            rest,
            Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
                span: sp,
            },
        ))
    } else {
        parse_comparison_expr(input, params, original)
    }
}

fn parse_comparison_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (mut rest, mut left) = parse_addition_expr(input, params, original)?;

    let (r, _) = ws(rest)?;

    // IS NULL / IS NOT NULL
    if let Ok((r2, _)) = keyword("IS")(r) {
        let (r3, _) = ws(r2)?;
        if let Ok((r4, _)) = keyword("NOT")(r3) {
            let (r5, _) = ws(r4)?;
            let (r6, _) = keyword("NULL")(r5)?;
            let sp = span_from(start, r6, original);
            return Ok((r6, Expr::IsNull {
                expr: Box::new(left),
                negated: true,
                span: sp,
            }));
        }
        let (r4, _) = keyword("NULL")(r3)?;
        let sp = span_from(start, r4, original);
        return Ok((r4, Expr::IsNull {
            expr: Box::new(left),
            negated: false,
            span: sp,
        }));
    }

    // IN
    if let Ok((r2, _)) = keyword("IN")(r) {
        let (r3, _) = ws(r2)?;
        let (r4, right) = parse_addition_expr(r3, params, original)?;
        let sp = span_from(start, r4, original);
        return Ok((r4, Expr::In {
            expr: Box::new(left),
            list: Box::new(right),
            negated: false,
            span: sp,
        }));
    }

    // NOT IN
    if let Ok((r2, _)) = keyword("NOT")(r) {
        let (r3, _) = ws(r2)?;
        if let Ok((r4, _)) = keyword("IN")(r3) {
            let (r5, _) = ws(r4)?;
            let (r6, right) = parse_addition_expr(r5, params, original)?;
            let sp = span_from(start, r6, original);
            return Ok((r6, Expr::In {
                expr: Box::new(left),
                list: Box::new(right),
                negated: true,
                span: sp,
            }));
        }
    }

    // STARTS WITH / ENDS WITH / CONTAINS
    if let Ok((r2, _)) = keyword("STARTS")(r) {
        let (r3, _) = multispace1(r2)?;
        let (r4, _) = keyword("WITH")(r3)?;
        let (r5, _) = ws(r4)?;
        let (r6, right) = parse_addition_expr(r5, params, original)?;
        let sp = span_from(start, r6, original);
        return Ok((r6, Expr::StringMatch {
            expr: Box::new(left),
            kind: StringMatchKind::StartsWith,
            pattern: Box::new(right),
            span: sp,
        }));
    }
    if let Ok((r2, _)) = keyword("ENDS")(r) {
        let (r3, _) = multispace1(r2)?;
        let (r4, _) = keyword("WITH")(r3)?;
        let (r5, _) = ws(r4)?;
        let (r6, right) = parse_addition_expr(r5, params, original)?;
        let sp = span_from(start, r6, original);
        return Ok((r6, Expr::StringMatch {
            expr: Box::new(left),
            kind: StringMatchKind::EndsWith,
            pattern: Box::new(right),
            span: sp,
        }));
    }
    if let Ok((r2, _)) = keyword("CONTAINS")(r) {
        let (r3, _) = ws(r2)?;
        let (r4, right) = parse_addition_expr(r3, params, original)?;
        let sp = span_from(start, r4, original);
        return Ok((r4, Expr::StringMatch {
            expr: Box::new(left),
            kind: StringMatchKind::Contains,
            pattern: Box::new(right),
            span: sp,
        }));
    }

    // Comparison operators: =, <>, <, >, <=, >=, =~
    let op = if let Ok((r2, _)) = try_tag("=~", r) {
        Some((r2, BinaryOp::RegexMatch))
    } else if let Ok((r2, _)) = try_tag("<>", r) {
        Some((r2, BinaryOp::Neq))
    } else if let Ok((r2, _)) = try_tag("<=", r) {
        Some((r2, BinaryOp::Lte))
    } else if let Ok((r2, _)) = try_tag(">=", r) {
        Some((r2, BinaryOp::Gte))
    } else if let Ok((r2, _)) = try_char('<', r) {
        Some((r2, BinaryOp::Lt))
    } else if let Ok((r2, _)) = try_char('>', r) {
        Some((r2, BinaryOp::Gt))
    } else if let Ok((r2, _)) = try_char('=', r) {
        // Make sure it's not == or => or =~
        if !r2.starts_with('=') && !r2.starts_with('>') && !r2.starts_with('~') {
            Some((r2, BinaryOp::Eq))
        } else {
            None
        }
    } else {
        None
    };

    if let Some((r2, op)) = op {
        let (r3, _) = ws(r2)?;
        let (r4, right) = parse_addition_expr(r3, params, original)?;
        let sp = span_from(start, r4, original);
        left = Expr::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
            span: sp,
        };
        rest = r4;
    }

    Ok((rest, left))
}

fn parse_addition_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (mut rest, mut left) = parse_multiplication_expr(input, params, original)?;
    loop {
        let (r, _) = ws(rest)?;
        let op = if let Ok((r2, _)) = try_char('+', r) {
            Some((r2, BinaryOp::Add))
        } else if let Ok((r2, _)) = try_char('-', r) {
            // Make sure it's not part of a relationship pattern ->
            if !r2.starts_with('>') && !r2.starts_with('[') {
                Some((r2, BinaryOp::Sub))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((r2, op)) = op {
            let (r3, _) = ws(r2)?;
            let (r4, right) = parse_multiplication_expr(r3, params, original)?;
            let sp = span_from(start, r4, original);
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span: sp,
            };
            rest = r4;
        } else {
            break;
        }
    }
    Ok((rest, left))
}

fn parse_multiplication_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (mut rest, mut left) = parse_unary_expr(input, params, original)?;
    loop {
        let (r, _) = ws(rest)?;
        let op = if let Ok((r2, _)) = try_char('*', r) {
            Some((r2, BinaryOp::Mul))
        } else if let Ok((r2, _)) = try_char('/', r) {
            Some((r2, BinaryOp::Div))
        } else if let Ok((r2, _)) = try_char('%', r) {
            Some((r2, BinaryOp::Mod))
        } else {
            None
        };
        if let Some((r2, op)) = op {
            let (r3, _) = ws(r2)?;
            let (r4, right) = parse_unary_expr(r3, params, original)?;
            let sp = span_from(start, r4, original);
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span: sp,
            };
            rest = r4;
        } else {
            break;
        }
    }
    Ok((rest, left))
}

fn parse_unary_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    if let Ok((rest, _)) = try_char('-', input) {
        // Negative sign — but not a relationship pattern
        if !rest.starts_with('[') && !rest.starts_with('>') && !rest.starts_with('-') {
            let (rest, _) = ws(rest)?;
            let (rest, expr) = parse_postfix_expr(rest, params, original)?;
            let sp = span_from(start, rest, original);
            return Ok((
                rest,
                Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                    span: sp,
                },
            ));
        }
    }
    parse_postfix_expr(input, params, original)
}

fn parse_postfix_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (mut rest, mut expr) = parse_atom(input, params, original)?;

    loop {
        // Property access: .name
        if let Ok((r, _)) = try_char('.', rest) {
            if let Ok((r2, name)) = symbolic_name(r) {
                let sp = span_from(start, r2, original);
                expr = Expr::Property {
                    expr: Box::new(expr),
                    name,
                    span: sp,
                };
                rest = r2;
                continue;
            }
        }
        // Index access: [expr]
        if let Ok((r, _)) = try_char('[', rest) {
            let (r2, _) = ws(r)?;
            let (r3, index) = parse_expr(r2, params, original)?;
            let (r4, _) = ws(r3)?;
            let (r5, _) = char(']')(r4)?;
            let sp = span_from(start, r5, original);
            expr = Expr::Index {
                expr: Box::new(expr),
                index: Box::new(index),
                span: sp,
            };
            rest = r5;
            continue;
        }
        break;
    }

    Ok((rest, expr))
}

fn parse_atom<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;

    // Parenthesized expression
    if let Ok((rest, _)) = try_char('(', input) {
        let (rest, _) = ws(rest)?;
        let (rest, expr) = parse_expr(rest, params, original)?;
        let (rest, _) = ws(rest)?;
        let (rest, _) = char(')')(rest)?;
        return Ok((rest, expr));
    }

    // $param
    if input.starts_with('$') {
        return parse_param(input, params, original);
    }

    // count(*)
    if let Ok((rest, _)) = keyword("count")(input) {
        let (rest, _) = ws(rest)?;
        if let Ok((rest, _)) = try_char('(', rest) {
            let (rest, _) = ws(rest)?;
            if let Ok((rest, _)) = try_char('*', rest) {
                let (rest, _) = ws(rest)?;
                let (rest, _) = char(')')(rest)?;
                let sp = span_from(start, rest, original);
                return Ok((rest, Expr::CountAll(sp)));
            }
        }
    }

    // CASE expression
    if let Ok((rest, _)) = keyword("CASE")(input) {
        return parse_case_expr(input, params, original);
    }

    // Boolean literals
    if let Ok((rest, _)) = keyword("TRUE")(input) {
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::Bool(true, sp)));
    }
    if let Ok((rest, _)) = keyword("FALSE")(input) {
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::Bool(false, sp)));
    }

    // NULL
    if let Ok((rest, _)) = keyword("NULL")(input) {
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::Null(sp)));
    }

    // String literal
    if input.starts_with('\'') || input.starts_with('"') {
        let (rest, val) = string_literal(input)?;
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::StringLit(val, sp)));
    }

    // List literal [...]
    if input.starts_with('[') {
        return parse_list_literal(input, params, original);
    }

    // Map literal {...}
    if input.starts_with('{') {
        let (rest, expr) = parse_map_literal(input, params, original)?;
        return Ok((rest, expr));
    }

    // Float literal (try before integer)
    if let Ok((rest, (val, _))) = float_literal(input) {
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::Float(val, sp)));
    }

    // Integer literal
    if input.starts_with(|c: char| c.is_ascii_digit()) {
        let (rest, (val, _)) = integer_literal(input)?;
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::Integer(val, sp)));
    }

    // Function call or identifier
    if let Ok((rest, name)) = symbolic_name(input) {
        let (r, _) = ws(rest)?;
        // Check if it's a function call
        if let Ok((r2, _)) = try_char('(', r) {
            let (r3, _) = ws(r2)?;
            // Check for DISTINCT
            let (r3, distinct) = if let Ok((r4, _)) = keyword("DISTINCT")(r3) {
                let (r5, _) = ws(r4)?;
                (r5, true)
            } else {
                (r3, false)
            };
            // Parse args
            if let Ok((r4, _)) = try_char(')', r3) {
                let sp = span_from(start, r4, original);
                return Ok((
                    r4,
                    Expr::FunctionCall {
                        name,
                        distinct,
                        args: vec![],
                        span: sp,
                    },
                ));
            }
            let (r4, first) = parse_expr(r3, params, original)?;
            let mut args = vec![first];
            let mut current = r4;
            while let Ok((r5, _)) = comma_sep(current) {
                let (r6, arg) = parse_expr(r5, params, original)?;
                args.push(arg);
                current = r6;
            }
            let (current, _) = ws(current)?;
            let (r5, _) = char(')')(current)?;
            let sp = span_from(start, r5, original);
            return Ok((
                r5,
                Expr::FunctionCall {
                    name,
                    distinct,
                    args,
                    span: sp,
                },
            ));
        }
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::Ident { name, span: sp }));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input,
        nom::error::ErrorKind::Alt,
    )))
}

fn parse_list_literal<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (rest, _) = char('[')(input)?;
    let (rest, _) = ws(rest)?;

    if let Ok((rest, _)) = try_char(']', rest) {
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::List(vec![], sp)));
    }

    let (rest, first) = parse_expr(rest, params, original)?;
    let mut items = vec![first];
    let mut current = rest;
    while let Ok((r, _)) = comma_sep(current) {
        let (r2, item) = parse_expr(r, params, original)?;
        items.push(item);
        current = r2;
    }
    let (current, _) = ws(current)?;
    let (rest, _) = char(']')(current)?;
    let sp = span_from(start, rest, original);
    Ok((rest, Expr::List(items, sp)))
}

fn parse_map_literal<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (rest, _) = char('{')(input)?;
    let (rest, _) = ws(rest)?;

    if let Ok((rest, _)) = try_char('}', rest) {
        let sp = span_from(start, rest, original);
        return Ok((rest, Expr::Map(vec![], sp)));
    }

    let (rest, first_key) = symbolic_name(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, _) = char(':')(rest)?;
    let (rest, _) = ws(rest)?;
    let (rest, first_val) = parse_expr(rest, params, original)?;
    let mut entries = vec![(first_key, first_val)];
    let mut current = rest;

    while let Ok((r, _)) = comma_sep(current) {
        let (r2, key) = symbolic_name(r)?;
        let (r3, _) = ws(r2)?;
        let (r3, _) = char(':')(r3)?;
        let (r4, _) = ws(r3)?;
        let (r5, val) = parse_expr(r4, params, original)?;
        entries.push((key, val));
        current = r5;
    }
    let (current, _) = ws(current)?;
    let (rest, _) = char('}')(current)?;
    let sp = span_from(start, rest, original);
    Ok((rest, Expr::Map(entries, sp)))
}

fn parse_case_expr<'a>(
    input: &'a str,
    params: &mut Vec<ParamRef>,
    original: &'a str,
) -> IResult<&'a str, Expr> {
    let start = input;
    let (rest, _) = keyword("CASE")(input)?;
    let (rest, _) = ws(rest)?;

    // Simple CASE vs generic CASE
    let (rest, operand) = if let Ok((_, _)) = keyword("WHEN")(rest) {
        (rest, None)
    } else {
        let (r, expr) = parse_expr(rest, params, original)?;
        let (r, _) = ws(r)?;
        (r, Some(Box::new(expr)))
    };

    let mut whens = Vec::new();
    let mut current = rest;
    while let Ok((r, _)) = keyword("WHEN")(current) {
        let (r, _) = ws(r)?;
        let (r, condition) = parse_expr(r, params, original)?;
        let (r, _) = ws(r)?;
        let (r, _) = keyword("THEN")(r)?;
        let (r, _) = ws(r)?;
        let (r, result) = parse_expr(r, params, original)?;
        let (r, _) = ws(r)?;
        whens.push((condition, result));
        current = r;
    }

    let (current, else_expr) = if let Ok((r, _)) = keyword("ELSE")(current) {
        let (r, _) = ws(r)?;
        let (r, expr) = parse_expr(r, params, original)?;
        let (r, _) = ws(r)?;
        (r, Some(Box::new(expr)))
    } else {
        (current, None)
    };

    let (rest, _) = keyword("END")(current)?;
    let sp = span_from(start, rest, original);
    Ok((
        rest,
        Expr::Case {
            operand,
            whens,
            else_expr,
            span: sp,
        },
    ))
}

// ─── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_match_return() {
        let result = parse_cypher("MATCH (n:Person) RETURN n");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert!(result.params.is_empty());
        let stmt = result.statement.unwrap();
        match &stmt.query {
            Query::Single(sq) => {
                assert_eq!(sq.clauses.len(), 2);
                assert!(matches!(&sq.clauses[0], Clause::Match { .. }));
                assert!(matches!(&sq.clauses[1], Clause::Return { .. }));
            }
            _ => panic!("expected single query"),
        }
    }

    #[test]
    fn parse_match_with_params() {
        let result = parse_cypher(
            "MATCH (c:Customer {id: $customer_id})-[:BOUGHT]->(p:Product) WHERE p.price > $min_price RETURN p.name, p.price",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.params.len(), 2);
        assert_eq!(result.params[0].name, "customer_id");
        assert_eq!(result.params[1].name, "min_price");
    }

    #[test]
    fn parse_create() {
        let result = parse_cypher(
            "CREATE (p:Product {name: $name, price: $price, category: $category}) RETURN p",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.params.len(), 3);
    }

    #[test]
    fn parse_unwind() {
        let result = parse_cypher(
            "UNWIND $items AS item MERGE (p:Product {sku: item.sku}) SET p.name = item.name RETURN count(p) AS processed",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.params.len(), 1);
        assert_eq!(result.params[0].name, "items");
    }

    #[test]
    fn parse_recommendations_query() {
        // Note: bare pattern predicates like `NOT (c)-[:BOUGHT]->(rec)` are not yet supported.
        // Use a property-based WHERE clause instead.
        let result = parse_cypher(
            "MATCH (c:Customer)-[:BOUGHT]->(p:Product)<-[:BOUGHT]-(other)-[:BOUGHT]->(rec) WHERE c.id = $customer_id RETURN rec.name, count(*) AS score ORDER BY score DESC LIMIT 5",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.params.len(), 1);
        assert_eq!(result.params[0].name, "customer_id");

        let stmt = result.statement.unwrap();
        match &stmt.query {
            Query::Single(sq) => {
                assert_eq!(sq.clauses.len(), 2); // MATCH + RETURN
                if let Clause::Return { order_by, limit, .. } = &sq.clauses[1] {
                    assert!(order_by.is_some());
                    assert!(limit.is_some());
                } else {
                    panic!("expected RETURN clause");
                }
            }
            _ => panic!("expected single query"),
        }
    }

    #[test]
    fn parse_with_clause() {
        let result = parse_cypher(
            "MATCH (c:Customer)-[:PURCHASED]->(p:Product) WITH c, count(p) AS purchaseCount WHERE purchaseCount > $min RETURN c.name",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.params.len(), 1);
    }

    #[test]
    fn parse_union() {
        let result = parse_cypher(
            "MATCH (n:Person) RETURN n.name UNION ALL MATCH (n:Company) RETURN n.name",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        match &result.statement.unwrap().query {
            Query::Union { all, .. } => assert!(*all),
            _ => panic!("expected union"),
        }
    }

    #[test]
    fn parse_optional_match() {
        let result = parse_cypher(
            "MATCH (n:Person) OPTIONAL MATCH (n)-[:KNOWS]->(m) RETURN n, m",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn parse_relationship_types() {
        let result = parse_cypher(
            "MATCH (a)-[r:KNOWS|LIKES]->(b) RETURN r",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn parse_variable_length_rel() {
        let result = parse_cypher(
            "MATCH (a)-[*1..3]->(b) RETURN a, b",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn parse_case_expression() {
        let result = parse_cypher(
            "MATCH (n) RETURN CASE WHEN n.age < 18 THEN 'minor' WHEN n.age >= 18 THEN 'adult' ELSE 'unknown' END AS category",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn parse_string_predicates() {
        let result = parse_cypher(
            "MATCH (n:Product) WHERE n.name STARTS WITH $prefix RETURN n",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert_eq!(result.params.len(), 1);
    }

    #[test]
    fn parse_delete() {
        let result = parse_cypher(
            "MATCH (n:Temp) DETACH DELETE n",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn parse_merge_with_on_create() {
        let result = parse_cypher(
            "MERGE (p:Product {sku: $sku}) ON CREATE SET p.created = timestamp() ON MATCH SET p.updated = timestamp() RETURN p",
        );
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn params_have_correct_spans() {
        let input = "MATCH (n) WHERE n.id = $my_id RETURN n";
        let result = parse_cypher(input);
        assert_eq!(result.params.len(), 1);
        let p = &result.params[0];
        assert_eq!(p.name, "my_id");
        assert_eq!(&input[p.span.start..p.span.end], "$my_id");
    }

    #[test]
    fn parse_is_null() {
        let result = parse_cypher("MATCH (n) WHERE n.email IS NOT NULL RETURN n");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn parse_list_and_in() {
        let result = parse_cypher("MATCH (n) WHERE n.status IN ['active', 'pending'] RETURN n");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }
}
