use winnow::ascii::space0;
use winnow::combinator::*;
use winnow::error::{ErrMode, InputError};
use winnow::prelude::*;
use winnow::token::{literal, take_while};

/// Canonical AQL field names. Every field the index understands is listed
/// here; anything else is rejected by [`validate_query`] with a helpful
/// message instead of panicking deeper in the query builder.
pub const VALID_FIELDS: &[&str] = &[
    "body",
    "category",
    "chat_role",
    "closed",
    "date",
    "deadline",
    "file_name",
    "id",
    "scheduled",
    "status",
    "tags",
    "title",
    "type",
];

/// User-facing aliases and the canonical field they resolve to. The parser
/// applies these before validation, so validation only ever sees canonical
/// names.
pub const FIELD_ALIASES: &[(&str, &str)] = &[("project", "category"), ("todo", "status")];

/// Fields that hold dates. Range queries are only meaningful on these, and
/// their values must be `YYYY-MM-DD`.
pub const DATE_FIELDS: &[&str] = &["date", "scheduled", "deadline", "closed"];

/// Values accepted by the `type` field. Mirrors `DocumentType`.
pub const DOCUMENT_TYPES: &[&str] = &["note", "task", "meeting", "heading", "chat"];

#[derive(Debug, PartialEq)]
pub enum RangeOp {
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, PartialEq)]
pub enum Expr {
    Term {
        field: Option<String>,
        value: String,
        phrase: bool,
        negated: bool,
    },
    Range {
        field: String,
        op: RangeOp,
        value: String,
        negated: bool,
    },
    FieldExists {
        field: String,
        negated: bool,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

/// Map user-facing field names to their canonical schema name.
///
/// Aliases are matched case-sensitively (lowercase only) to match
/// the existing field-name convention. Add new aliases here; downstream
/// consumers see only canonical names.
fn resolve_field_alias(field: &str) -> &str {
    match field {
        "project" => "category",
        "todo" => "status",
        _ => field,
    }
}

/// Parse an AQL query into an [`Expr`].
///
/// This only checks syntax. Call [`validate_query`] (or
/// [`parse_and_validate_query`]) before handing the expression to the
/// index, which assumes every field name is real and every range value is
/// a date.
pub fn parse_query(input: &str) -> anyhow::Result<Expr> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!(
            "query is empty — provide a search term such as `meeting`, `title:rust`, \
             or `status:todo deadline:<=2025-01-31`"
        );
    }

    let mut rest = trimmed;
    let expr = parse_expr(&mut rest).map_err(|e| {
        let remaining = match &e {
            ErrMode::Backtrack(err) | ErrMode::Cut(err) => err.input,
            ErrMode::Incomplete(_) => trimmed,
        };
        let position = trimmed.len().saturating_sub(remaining.len());
        format_parse_error(position, remaining)
    })?;

    let trailing = rest.trim_start();
    if !trailing.is_empty() {
        let position = trimmed.len().saturating_sub(trailing.len());
        anyhow::bail!(
            "unexpected input at position {position} near `{}`. Check for unbalanced \
             quotes or unsupported characters such as parentheses. A query is a list of \
             space-separated terms, optionally joined with `AND` / `OR`.",
            preview(trailing)
        );
    }

    Ok(expr)
}

/// Parse and validate an AQL query. Use this at the boundary between user
/// input and the index: it rejects unknown fields, invalid `type` values,
/// malformed dates, and ranges on non-date fields.
pub fn parse_and_validate_query(input: &str) -> anyhow::Result<Expr> {
    let expr = parse_query(input)?;
    validate_query(&expr)?;
    Ok(expr)
}

/// Check that a parsed expression only references fields and values the
/// index understands.
///
/// The parser is deliberately permissive about identifiers; validation is
/// what turns a typo like `titel:rust` or `date:>yesterday` into an error
/// the caller (and the model) can act on.
pub fn validate_query(expr: &Expr) -> anyhow::Result<()> {
    match expr {
        Expr::Term {
            field: Some(field),
            value,
            phrase,
            ..
        } => {
            validate_field(field)?;
            validate_quotes(value, *phrase)?;
            if field == "type" {
                validate_type_value(value)?;
            }
            if DATE_FIELDS.contains(&field.as_str()) {
                validate_date(value, field)?;
            }
            Ok(())
        }
        Expr::Term { value, phrase, .. } => validate_quotes(value, *phrase),
        Expr::Range { field, value, .. } => {
            validate_field(field)?;
            if !DATE_FIELDS.contains(&field.as_str()) {
                anyhow::bail!(
                    "range queries (>, >=, <, <=) are only supported on date fields ({}), \
                     but `{field}` is not a date field",
                    DATE_FIELDS.join(", ")
                );
            }
            validate_date(value, field)
        }
        Expr::FieldExists { field, .. } => validate_field(field),
        Expr::And(left, right) | Expr::Or(left, right) => {
            validate_query(left)?;
            validate_query(right)
        }
    }
}

fn validate_field(field: &str) -> anyhow::Result<()> {
    if VALID_FIELDS.contains(&field) {
        return Ok(());
    }

    // A case-only typo is common (`Title:rust`); point at the right spelling.
    if let Some(suggestion) = VALID_FIELDS
        .iter()
        .find(|valid| valid.eq_ignore_ascii_case(field))
    {
        anyhow::bail!(
            "unknown field `{field}` (field names are lowercase — did you mean `{suggestion}`?)"
        );
    }

    let aliases = FIELD_ALIASES
        .iter()
        .map(|(alias, canonical)| format!("`{alias}` (→ `{canonical}`)"))
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::bail!(
        "unknown field `{field}`. Valid fields: {}. Aliases: {aliases}",
        VALID_FIELDS.join(", ")
    )
}

fn validate_quotes(value: &str, phrase: bool) -> anyhow::Result<()> {
    if !phrase && value.contains('"') {
        anyhow::bail!(
            "unbalanced quote in `{value}`. Wrap a phrase in a pair of double quotes, \
             e.g. `title:\"rust programming\"`"
        );
    }
    Ok(())
}

fn validate_type_value(value: &str) -> anyhow::Result<()> {
    if DOCUMENT_TYPES
        .iter()
        .any(|valid| valid.eq_ignore_ascii_case(value))
    {
        return Ok(());
    }
    anyhow::bail!(
        "invalid value `{value}` for field `type`. Expected one of: {}",
        DOCUMENT_TYPES.join(", ")
    )
}

fn validate_date(value: &str, field: &str) -> anyhow::Result<()> {
    let bytes = value.as_bytes();
    let valid_format = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit);
    // Slicing is safe here: `valid_format` guarantees ASCII digits at these
    // positions, so they are char boundaries.
    let valid_month_day = valid_format
        && (1..=12).contains(&value[5..7].parse::<u32>().unwrap_or(0))
        && (1..=31).contains(&value[8..].parse::<u32>().unwrap_or(0));

    if valid_month_day {
        return Ok(());
    }
    anyhow::bail!(
        "invalid date `{value}` for field `{field}`. Use `YYYY-MM-DD`, \
         e.g. `{field}:2025-01-31` or `{field}:>=2025-01-01`"
    )
}

fn preview(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(20).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn format_parse_error(position: usize, remaining: &str) -> anyhow::Error {
    let near = if remaining.trim().is_empty() {
        String::from("end of query")
    } else {
        format!("near `{}`", preview(remaining))
    };
    // `AND`, `OR`, and `NOT` are reserved. A lone one is a common way to
    // search for the literal word, so point at quoting.
    let reserved_hint = remaining
        .split_whitespace()
        .next()
        .filter(|word| {
            ["AND", "OR", "NOT"]
                .iter()
                .any(|op| word.eq_ignore_ascii_case(op))
        })
        .map(|word| {
            format!(
                " `{word}` is a reserved operator; quote it as `\"{word}\"` to \
                 search for the word itself."
            )
        })
        .unwrap_or_default();

    anyhow::anyhow!(
        "failed to parse query at position {position} ({near}). Expected a term \
         (`rust`), a fielded term (`title:rust`), a quoted phrase \
         (`title:\"rust programming\"`), a range (`date:>2025-01-01`), or a \
         negation (`-status:done`).{reserved_hint}"
    )
}

fn parse_expr<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    parse_or(input)
}

fn parse_or<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let mut lhs = parse_and(input)?;

    loop {
        // Peek for `OR` without consuming anything on failure.
        let Ok((rest, _)) = preceded(space0, tag_no_case("OR")).parse_peek(*input) else {
            break;
        };
        *input = rest;
        let _ = space0.parse_next(input)?;
        let rhs = parse_and(input)?;
        lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
    }

    Ok(lhs)
}

fn parse_and<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let mut lhs = parse_not(input)?;

    loop {
        // Explicit `AND` between operands.
        if let Ok((rest, _)) = preceded(space0, tag_no_case("AND")).parse_peek(*input) {
            *input = rest;
            let _ = space0.parse_next(input)?;
            let rhs = parse_not(input)?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
            continue;
        }

        // Stop before an explicit `OR` so `parse_or` can build the
        // lower-precedence node.
        if preceded(space0, tag_no_case("OR"))
            .parse_peek(*input)
            .is_ok()
        {
            break;
        }

        // Implicit AND: skip whitespace, then parse another term.
        let before_whitespace = *input;
        let _ = space0.parse_next(input)?;
        if input.is_empty() {
            break;
        }
        match parse_not(input) {
            Ok(rhs) => lhs = Expr::And(Box::new(lhs), Box::new(rhs)),
            Err(_) => {
                *input = before_whitespace;
                break;
            }
        }
    }

    Ok(lhs)
}

fn parse_not<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let negated = opt(alt((literal("-"), tag_no_case("NOT"))))
        .parse_next(input)?
        .is_some();
    if negated {
        // `NOT status:done` is equivalent to `-status:done`.
        let _ = space0.parse_next(input)?;
    }
    let mut expr = parse_term(input)?;
    match &mut expr {
        Expr::Term { negated: n, .. } => *n = *n || negated,
        Expr::Range { negated: n, .. } => *n = *n || negated,
        Expr::FieldExists { negated: n, .. } => *n = *n || negated,
        _ => (),
    }

    Ok(expr)
}

fn parse_term<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    alt((parse_range_expr, parse_fielded_term, parse_default_term)).parse_next(input)
}

/// A field name: ASCII alphanumerics and underscores (`file_name`,
/// `chat_role`). Underscores are allowed so every schema field is
/// addressable; without this they would parse as default terms.
fn field_name<'a>(input: &mut &'a str) -> Result<&'a str, ErrMode<InputError<&'a str>>> {
    take_while(1.., |c: char| c.is_ascii_alphanumeric() || c == '_').parse_next(input)
}

fn parse_range_expr<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let negated = opt(literal("-")).parse_next(input)?.is_some();
    let field: &str = field_name(input)?;
    literal(":").parse_next(input)?;
    let op = alt((
        literal(">=").map(|_| RangeOp::Gte),
        literal("<=").map(|_| RangeOp::Lte),
        literal(">").map(|_| RangeOp::Gt),
        literal("<").map(|_| RangeOp::Lt),
    ))
    .parse_next(input)?;
    let value = take_while(1.., |c: char| !c.is_whitespace() && c != ')').parse_next(input)?;
    Ok(Expr::Range {
        field: resolve_field_alias(field).to_string(),
        op,
        value: value.to_string(),
        negated,
    })
}

fn parse_fielded_term<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let negated = opt(literal("-")).parse_next(input)?.is_some();
    let field_raw: &str = field_name(input)?;
    literal(":").parse_next(input)?;

    let field = resolve_field_alias(field_raw);

    // `field:` with no value: end-of-input, whitespace, or `)` means
    // "filter for documents that have any non-null value in this field".
    let next_is_stopper = input
        .chars()
        .next()
        .map(|c| c.is_whitespace() || c == ')')
        .unwrap_or(true);
    if next_is_stopper {
        return Ok(Expr::FieldExists {
            field: field.to_string(),
            negated,
        });
    }

    let term_parser = alt((
        delimited(literal("\""), take_while(1.., |c| c != '"'), literal("\""))
            .map(|s: &str| (s.to_string(), true)),
        take_while(1.., |c: char| !c.is_whitespace() && c != ')' && c != ',')
            .map(|s: &str| (s.to_string(), false)),
    ));

    let values: Vec<(String, bool)> =
        separated(1.., term_parser, literal(",")).parse_next(input)?;

    if values.len() == 1 {
        Ok(Expr::Term {
            field: Some(field.to_string()),
            value: values[0].0.clone(),
            phrase: values[0].1,
            negated,
        })
    } else {
        // A comma list is an OR of the values — matching org-ql, where
        // comma-separated arguments to a predicate match "one or more of".
        // AND is expressed by repeating the field: `tags:a tags:b`.
        let mut terms = values.into_iter().map(|(value, phrase)| Expr::Term {
            field: Some(field.to_string()),
            value,
            phrase,
            negated,
        });
        let first = terms.next().unwrap();
        Ok(terms.fold(first, |acc, term| Expr::Or(Box::new(acc), Box::new(term))))
    }
}

fn parse_default_term<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let start = *input;
    let value = alt((
        delimited(literal("\""), take_while(1.., |c| c != '"'), literal("\""))
            .map(|s: &str| (s.to_string(), true)),
        take_while(1.., |c: char| !c.is_whitespace() && c != ')')
            .map(|s: &str| (s.to_string(), false)),
    ))
    .parse_next(input)?;

    // `AND` and `OR` are operators, not search terms. Reject them here so
    // the combinators above can see them; a user who really means the word
    // can quote it (`"AND"`).
    if !value.1 && (value.0.eq_ignore_ascii_case("AND") || value.0.eq_ignore_ascii_case("OR")) {
        return Err(ErrMode::Backtrack(InputError::at(start)));
    }

    Ok(Expr::Term {
        field: None,
        value: value.0,
        phrase: value.1,
        negated: false,
    })
}

fn tag_no_case<'a>(
    tag_str: &'static str,
) -> impl Parser<&'a str, &'a str, ErrMode<InputError<&'a str>>> {
    move |input: &mut &'a str| {
        let len = tag_str.len();
        let (head, tail) = input.split_at(len.min(input.len()));
        // Require a word boundary so `ORANGE` and `NOTES` are terms, not
        // the `OR` / `NOT` operators.
        let at_boundary = tail
            .chars()
            .next()
            .map(|c| c.is_whitespace() || c == ')')
            .unwrap_or(true);
        if head.eq_ignore_ascii_case(tag_str) && at_boundary {
            *input = tail;
            Ok(head)
        } else {
            Err(ErrMode::Backtrack(InputError::at(*input)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range() {
        let result = parse_query("date:>2024-01-01").unwrap();
        assert_eq!(
            result,
            Expr::Range {
                field: "date".into(),
                op: RangeOp::Gt,
                value: "2024-01-01".into(),
                negated: false,
            }
        );
    }

    #[test]
    fn test_negated_range() {
        let result = parse_query("-price:<=100").unwrap();
        assert_eq!(
            result,
            Expr::Range {
                field: "price".into(),
                op: RangeOp::Lte,
                value: "100".into(),
                negated: true,
            }
        );
    }

    #[test]
    fn test_multiple_terms() {
        let result = parse_query("title:testing tags:meeting date:>2025-01-01").unwrap();
        assert_eq!(
            result,
            Expr::And(
                Box::new(Expr::And(
                    Box::new(Expr::Term {
                        field: Some(String::from("title")),
                        value: String::from("testing"),
                        phrase: false,
                        negated: false
                    }),
                    Box::new(Expr::Term {
                        field: Some(String::from("tags")),
                        value: String::from("meeting"),
                        phrase: false,
                        negated: false
                    })
                )),
                Box::new(Expr::Range {
                    field: String::from("date"),
                    op: RangeOp::Gt,
                    value: String::from("2025-01-01"),
                    negated: false
                })
            )
        );
    }

    #[test]
    fn test_comma_separated_terms_is_or() {
        // Comma means OR for any field (matching org-ql). `tags:work,urgent`
        // matches docs with tag "work" OR tag "urgent".
        let result = parse_query("tags:work,urgent").unwrap();
        assert_eq!(
            result,
            Expr::Or(
                Box::new(Expr::Term {
                    field: Some(String::from("tags")),
                    value: String::from("work"),
                    phrase: false,
                    negated: false
                }),
                Box::new(Expr::Term {
                    field: Some(String::from("tags")),
                    value: String::from("urgent"),
                    phrase: false,
                    negated: false
                })
            ),
        );
    }

    #[test]
    fn test_space_separated_same_field_is_and() {
        // Repeating the field with a space means AND: `tags:a tags:b`
        // requires the doc to have both tags.
        let result = parse_query("tags:work tags:urgent").unwrap();
        assert_eq!(
            result,
            Expr::And(
                Box::new(Expr::Term {
                    field: Some(String::from("tags")),
                    value: String::from("work"),
                    phrase: false,
                    negated: false
                }),
                Box::new(Expr::Term {
                    field: Some(String::from("tags")),
                    value: String::from("urgent"),
                    phrase: false,
                    negated: false
                })
            ),
        );
    }

    #[test]
    fn test_comma_separated_terms_single_value_field_is_or() {
        // `status` is single-valued, so a comma list means "any of these".
        // `todo:next,todo` should NOT become status:next AND status:todo
        // (which could never match), but an OR of the two.
        let result = parse_query("todo:next,todo").unwrap();
        assert_eq!(
            result,
            Expr::Or(
                Box::new(Expr::Term {
                    field: Some(String::from("status")),
                    value: String::from("next"),
                    phrase: false,
                    negated: false
                }),
                Box::new(Expr::Term {
                    field: Some(String::from("status")),
                    value: String::from("todo"),
                    phrase: false,
                    negated: false
                })
            ),
        );
    }

    #[test]
    fn test_comma_separated_three_values_single_value_field_is_or() {
        let result = parse_query("todo:next,todo,done").unwrap();
        assert_eq!(
            result,
            Expr::Or(
                Box::new(Expr::Or(
                    Box::new(Expr::Term {
                        field: Some(String::from("status")),
                        value: String::from("next"),
                        phrase: false,
                        negated: false
                    }),
                    Box::new(Expr::Term {
                        field: Some(String::from("status")),
                        value: String::from("todo"),
                        phrase: false,
                        negated: false
                    })
                )),
                Box::new(Expr::Term {
                    field: Some(String::from("status")),
                    value: String::from("done"),
                    phrase: false,
                    negated: false
                })
            ),
        );
    }

    #[test]
    fn test_field_alias_project_resolves_to_category() {
        let result = parse_query("project:work").unwrap();
        assert_eq!(
            result,
            Expr::Term {
                field: Some(String::from("category")),
                value: String::from("work"),
                phrase: false,
                negated: false
            }
        );
    }

    #[test]
    fn test_field_alias_todo_resolves_to_status() {
        let result = parse_query("todo:done").unwrap();
        assert_eq!(
            result,
            Expr::Term {
                field: Some(String::from("status")),
                value: String::from("done"),
                phrase: false,
                negated: false
            }
        );
    }

    #[test]
    fn test_field_alias_applied_to_range_expr() {
        let result = parse_query("project:>2024-01-01").unwrap();
        assert_eq!(
            result,
            Expr::Range {
                field: String::from("category"),
                op: RangeOp::Gt,
                value: String::from("2024-01-01"),
                negated: false
            }
        );
    }

    #[test]
    fn test_unaliased_field_passes_through() {
        // `category` is the canonical name — it should not be remapped.
        let result = parse_query("category:work").unwrap();
        assert_eq!(
            result,
            Expr::Term {
                field: Some(String::from("category")),
                value: String::from("work"),
                phrase: false,
                negated: false
            }
        );
    }

    #[test]
    fn test_field_exists_at_end_of_input() {
        let result = parse_query("todo:").unwrap();
        assert_eq!(
            result,
            Expr::FieldExists {
                field: String::from("status"),
                negated: false
            }
        );
    }

    #[test]
    fn test_field_exists_followed_by_whitespace() {
        // `todo: bar` — FieldExists for status, then default term "bar"
        let result = parse_query("todo: bar").unwrap();
        assert_eq!(
            result,
            Expr::And(
                Box::new(Expr::FieldExists {
                    field: String::from("status"),
                    negated: false
                }),
                Box::new(Expr::Term {
                    field: None,
                    value: String::from("bar"),
                    phrase: false,
                    negated: false
                })
            )
        );
    }

    #[test]
    fn test_negated_field_exists() {
        let result = parse_query("-todo:").unwrap();
        assert_eq!(
            result,
            Expr::FieldExists {
                field: String::from("status"),
                negated: true
            }
        );
    }

    #[test]
    fn test_trailing_parenthesis_is_rejected() {
        // Parentheses are not part of AQL. The parser stops at `)`, and the
        // trailing-input check reports it instead of silently ignoring it.
        let result = parse_query("todo:)");
        assert!(result.is_err(), "`)` should be rejected: {result:?}");
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("unexpected input"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn test_empty_query_is_rejected_with_help() {
        let message = parse_query("   ").unwrap_err().to_string();
        assert!(message.contains("query is empty"), "got: {message}");
    }

    #[test]
    fn test_underscore_field_names_parse() {
        let result = parse_query("file_name:notes.org").unwrap();
        assert_eq!(
            result,
            Expr::Term {
                field: Some(String::from("file_name")),
                value: String::from("notes.org"),
                phrase: false,
                negated: false,
            }
        );
    }

    #[test]
    fn test_explicit_and() {
        let result = parse_query("status:todo AND deadline:2025-01-01").unwrap();
        assert_eq!(
            result,
            Expr::And(
                Box::new(Expr::Term {
                    field: Some(String::from("status")),
                    value: String::from("todo"),
                    phrase: false,
                    negated: false,
                }),
                Box::new(Expr::Term {
                    field: Some(String::from("deadline")),
                    value: String::from("2025-01-01"),
                    phrase: false,
                    negated: false,
                }),
            )
        );
    }

    #[test]
    fn test_explicit_or() {
        let result = parse_query("status:done OR status:canceled").unwrap();
        assert_eq!(
            result,
            Expr::Or(
                Box::new(Expr::Term {
                    field: Some(String::from("status")),
                    value: String::from("done"),
                    phrase: false,
                    negated: false,
                }),
                Box::new(Expr::Term {
                    field: Some(String::from("status")),
                    value: String::from("canceled"),
                    phrase: false,
                    negated: false,
                }),
            )
        );
    }

    #[test]
    fn test_and_binds_tighter_than_or() {
        let result = parse_query("a OR b AND c").unwrap();
        assert_eq!(
            result,
            Expr::Or(
                Box::new(Expr::Term {
                    field: None,
                    value: String::from("a"),
                    phrase: false,
                    negated: false,
                }),
                Box::new(Expr::And(
                    Box::new(Expr::Term {
                        field: None,
                        value: String::from("b"),
                        phrase: false,
                        negated: false,
                    }),
                    Box::new(Expr::Term {
                        field: None,
                        value: String::from("c"),
                        phrase: false,
                        negated: false,
                    }),
                )),
            )
        );
    }

    #[test]
    fn test_lone_operator_is_rejected_with_quote_hint() {
        let message = parse_query("AND").unwrap_err().to_string();
        assert!(message.contains("reserved operator"), "got: {message}");
        assert!(message.contains("\"AND\""), "got: {message}");
    }

    #[test]
    fn test_operator_words_are_not_terms() {
        // `ORANGE` starts with `OR` but is a term; the word-boundary check
        // keeps it from being split into an operator plus `ANGE`.
        let result = parse_query("ORANGE").unwrap();
        assert_eq!(
            result,
            Expr::Term {
                field: None,
                value: String::from("ORANGE"),
                phrase: false,
                negated: false,
            }
        );
    }

    #[test]
    fn test_validate_rejects_unknown_field() {
        let expr = parse_query("titel:rust").unwrap();
        let message = validate_query(&expr).unwrap_err().to_string();
        assert!(message.contains("unknown field `titel`"), "got: {message}");
        assert!(message.contains("title"), "got: {message}");
    }

    #[test]
    fn test_validate_suggests_lowercase_field() {
        let expr = parse_query("Title:rust").unwrap();
        let message = validate_query(&expr).unwrap_err().to_string();
        assert!(message.contains("did you mean `title`"), "got: {message}");
    }

    #[test]
    fn test_validate_rejects_invalid_type_value() {
        let expr = parse_query("type:notes").unwrap();
        let message = validate_query(&expr).unwrap_err().to_string();
        assert!(
            message.contains("invalid value `notes` for field `type`"),
            "got: {message}"
        );
        assert!(message.contains("meeting"), "got: {message}");
    }

    #[test]
    fn test_validate_rejects_range_on_text_field() {
        let expr = parse_query("title:>2025-01-01").unwrap();
        let message = validate_query(&expr).unwrap_err().to_string();
        assert!(
            message.contains("only supported on date fields"),
            "got: {message}"
        );
    }

    #[test]
    fn test_validate_rejects_malformed_date() {
        let expr = parse_query("deadline:>tomorrow").unwrap();
        let message = validate_query(&expr).unwrap_err().to_string();
        assert!(
            message.contains("invalid date `tomorrow`"),
            "got: {message}"
        );
    }

    #[test]
    fn test_validate_accepts_realistic_query() {
        let expr = parse_and_validate_query(
            "type:task status:todo -status:done deadline:<=2025-01-31 tags:work",
        )
        .unwrap();
        assert!(matches!(expr, Expr::And(_, _)));
    }

    #[test]
    fn test_validate_rejects_unbalanced_quote() {
        let expr = parse_query("title:\"rust").unwrap();
        let message = validate_query(&expr).unwrap_err().to_string();
        assert!(message.contains("unbalanced quote"), "got: {message}");
    }

    #[test]
    fn test_field_exists_unaliased_field() {
        // A non-aliased field with no value should also produce FieldExists.
        let result = parse_query("category:").unwrap();
        assert_eq!(
            result,
            Expr::FieldExists {
                field: String::from("category"),
                negated: false
            }
        );
    }
}
