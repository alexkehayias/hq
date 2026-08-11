use winnow::ascii::{alphanumeric1, space0};
use winnow::combinator::*;
use winnow::error::{ErrMode, InputError};
use winnow::prelude::*;
use winnow::token::{literal, take_while};

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

pub fn parse_query(input: &str) -> Result<Expr, ErrMode<InputError<&str>>> {
    let mut input = input;
    parse_expr(&mut input)
}

fn parse_expr<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    parse_or(input)
}

fn parse_or<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let mut lhs = parse_and(input)?;
    while preceded(space0, tag_no_case("OR"))
        .parse_next(input)
        .is_ok()
    {
        let rhs = parse_and(input)?;
        lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
    }
    Ok(lhs)
}

fn parse_and<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let mut lhs = parse_not(input)?;

    loop {
        let checkpoint = *input;

        if preceded(space0, tag_no_case("AND"))
            .parse_next(input)
            .is_ok()
        {
            *input = checkpoint;
            break;
        }

        if let Ok(rhs) = parse_not(input) {
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        } else {
            break;
        }
    }

    Ok(lhs)
}

fn parse_not<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let negated = opt(alt((literal("-"), tag_no_case("NOT"))))
        .parse_next(input)?
        .is_some();
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

fn parse_range_expr<'a>(input: &mut &'a str) -> Result<Expr, ErrMode<InputError<&'a str>>> {
    let negated = opt(literal("-")).parse_next(input)?.is_some();
    let field: &str = alphanumeric1.parse_next(input)?;
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
    let field_raw: &str = alphanumeric1.parse_next(input)?;
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
    let value = alt((
        delimited(literal("\""), take_while(1.., |c| c != '"'), literal("\""))
            .map(|s: &str| (s.to_string(), true)),
        take_while(1.., |c: char| !c.is_whitespace() && c != ')')
            .map(|s: &str| (s.to_string(), false)),
    ))
    .parse_next(input)?;
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
        if head.eq_ignore_ascii_case(tag_str) {
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
    fn test_field_exists_before_closing_paren() {
        // `)` is a value stopper in the existing parser, so `(todo:)`
        // should parse as FieldExists rather than failing.
        let result = parse_query("todo:)");
        // The `)` is left unconsumed in the input — only FieldExists
        // should be produced from the `todo:` portion.
        assert!(result.is_ok(), "parse should succeed: {:?}", result);
        let expr = result.unwrap();
        assert_eq!(
            expr,
            Expr::FieldExists {
                field: String::from("status"),
                negated: false
            }
        );
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
