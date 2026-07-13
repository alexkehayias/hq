use crate::search::aql::{Expr, RangeOp};
use std::ops::Bound;
use tantivy::Index;
use tantivy::Term;
use tantivy::query::{AllQuery, BooleanQuery, FuzzyTermQuery, PhraseQuery, RegexQuery, TermQuery};
use tantivy::query::{Occur, Query};
use tantivy::schema::{Field, IndexRecordOption, Schema};

fn parse_date_to_timestamp(date_str: &str) -> u64 {
    let parts: Vec<u32> = date_str.split('-').map(|s| s.parse().unwrap()).collect();
    let (year, month, day) = (parts[0], parts[1], parts[2]);

    // Calculate days since UNIX_EPOCH
    let days = (year as i64 - 1970) * 365 + ((year as i64 - 1969) / 4)
        - ((year as i64 - 1901) / 100)
        + ((year as i64 - 1601) / 400)
        + match month {
            1 => 0,
            2 => 31,
            3 => 59,
            4 => 90,
            5 => 120,
            6 => 151,
            7 => 181,
            8 => 212,
            9 => 243,
            10 => 273,
            11 => 304,
            12 => 334,
            _ => 0,
        } as i64
        + day as i64
        - 1;

    (days * 24 * 60 * 60) as u64
}

const DEFAULT_FIELD_NAME: &str = "__default";

/// Only apply fuzzy search to title field for typo tolerance
/// otherwise the search hits become less useful
fn is_fuzzy_search_field(field: &str) -> bool {
    matches!(field, "title")
}

fn is_sql_only_field(field: &str) -> bool {
    matches!(field, "scheduled" | "deadline" | "closed" | "date")
}

/// Choose a fuzzy edit distance based on term length.
///
/// Short terms (≤4 chars) get distance 1 because distance 2 on a
/// 3-4 char abbreviation matches too many unrelated terms and drowns
/// out BM25 scores. Longer terms keep distance 2 for typo tolerance.
fn fuzzy_distance(term: &str) -> u8 {
    if term.len() <= 4 { 1 } else { 2 }
}

/// Tokenize `text` using the field's analyzer so query tokens match
/// what was indexed. Tantivy TEXT fields apply `SimpleTokenizer` +
/// `LowerCaser`, so without this step a query for "Lee" never matches
/// the indexed token "lee".
fn tokenize_value(idx: &Index, field: Field, text: &str) -> Vec<String> {
    let mut analyzer = idx.tokenizer_for_field(field).expect("No tokenizer for field");
    let mut stream = analyzer.token_stream(text);
    let mut out = Vec::new();
    while let Some(t) = stream.next() {
        out.push(t.text.clone());
    }
    out
}

/// Build a single-token query for one token of a tokenized value.
///
/// Fuzzy matching (typo tolerance) is only applied when `is_fuzzy` is
/// true, which the caller sets based on whether the field is a fuzzy
/// search field (title only — see `is_fuzzy_search_field`).
fn build_token_query(field: Field, is_fuzzy: bool, token: &str) -> Box<dyn Query> {
    let term = Term::from_field_text(field, token);
    if is_fuzzy {
        Box::new(FuzzyTermQuery::new(term, fuzzy_distance(token), true))
    } else {
        Box::new(TermQuery::new(term, IndexRecordOption::Basic))
    }
}

/// Build the query for a single field using tokenized text.
///
/// The analyzer lowercases and splits on punctuation the same way
/// the indexer does, so query tokens match what's in the dictionary.
/// Multi-token non-phrase values become a `BooleanQuery(Should)` of
/// per-token queries so docs containing any token match. Phrases use
/// `PhraseQuery` with lowercased tokens and slop 2.
fn build_field_query(
    idx: &Index,
    query_field: Field,
    query_field_name: &str,
    value: &str,
    phrase: bool,
    negated: bool,
) -> Option<Box<dyn Query>> {
    let tokens = tokenize_value(idx, query_field, value);
    if tokens.is_empty() {
        return None;
    }
    let is_fuzzy = is_fuzzy_search_field(query_field_name);

    if phrase {
        let terms: Vec<Term> = tokens
            .iter()
            .map(|t| Term::from_field_text(query_field, t))
            .collect();
        let mut phrase_q = PhraseQuery::new(terms);
        phrase_q.set_slop(2);
        if negated {
            Some(Box::new(BooleanQuery::new(vec![
                (Occur::Must, Box::new(AllQuery)),
                (
                    Occur::MustNot,
                    Box::new(phrase_q) as Box<dyn Query>,
                ),
            ])))
        } else {
            Some(Box::new(phrase_q))
        }
    } else if negated {
        // Exclude docs containing ANY token of the value. By
        // De Morgan, NOT (a OR b) == NOT a AND NOT b, so a
        // single MustNot over a Should clause is equivalent.
        let inner: Box<dyn Query> = if tokens.len() == 1 {
            build_token_query(query_field, is_fuzzy, &tokens[0])
        } else {
            Box::new(BooleanQuery::from(
                tokens
                    .iter()
                    .map(|t| (Occur::Should, build_token_query(query_field, is_fuzzy, t)))
                    .collect::<Vec<(Occur, Box<dyn Query>)>>(),
            ))
        };
        Some(Box::new(BooleanQuery::new(vec![
            (Occur::Must, Box::new(AllQuery)),
            (Occur::MustNot, inner),
        ])))
    } else {
        // Non-negated: match docs containing any token.
        if tokens.len() == 1 {
            Some(build_token_query(query_field, is_fuzzy, &tokens[0]))
        } else {
            Some(Box::new(BooleanQuery::from(
                tokens
                    .iter()
                    .map(|t| (Occur::Should, build_token_query(query_field, is_fuzzy, t)))
                    .collect::<Vec<(Occur, Box<dyn Query>)>>(),
            )))
        }
    }
}

pub fn aql_to_index_query(
    idx: &Index,
    schema: &Schema,
    expr: &Expr,
) -> Option<Box<dyn Query>> {
    match expr {
        Expr::Term {
            field: Some(field), ..
        } if is_sql_only_field(field) => None,
        Expr::Range { field, .. } if is_sql_only_field(field) => None,
        Expr::Term {
            field,
            value,
            phrase,
            negated,
        } => {
            // Default to title and body when there is no field name specified
            let field_name = field.clone().unwrap_or_else(|| "__default".into());
            let fields: Vec<(String, Field)> = if field_name == DEFAULT_FIELD_NAME {
                vec![
                    (String::from("title"), schema.get_field("title").unwrap()),
                    (String::from("body"), schema.get_field("body").unwrap()),
                ]
            } else {
                vec![(field_name.clone(), schema.get_field(&field_name).unwrap())]
            };

            let terms: Vec<Box<dyn Query>> = fields
                .iter()
                .filter_map(|(query_field_name, query_field)| {
                    build_field_query(
                        idx,
                        *query_field,
                        query_field_name,
                        value,
                        *phrase,
                        *negated,
                    )
                })
                .collect();

            if terms.is_empty() {
                None
            } else if terms.len() > 1 {
                Some(Box::new(BooleanQuery::from(
                    terms
                        .into_iter()
                        .map(|q| (Occur::Should, q))
                        .collect::<Vec<(Occur, Box<dyn Query>)>>(),
                )))
            } else {
                Some(terms.into_iter().next().unwrap())
            }
        }
        Expr::Range {
            field,
            op,
            value,
            negated,
        } => {
            let field = schema.get_field(field).unwrap();
            let value = parse_date_to_timestamp(value);
            let (lower_bound, upper_bound) = match op {
                RangeOp::Lt => (
                    Bound::Unbounded,
                    Bound::Excluded(Term::from_field_u64(field, value)),
                ),
                RangeOp::Lte => (
                    Bound::Unbounded,
                    Bound::Included(Term::from_field_u64(field, value)),
                ),
                RangeOp::Gt => (
                    Bound::Excluded(Term::from_field_u64(field, value)),
                    Bound::Unbounded,
                ),
                RangeOp::Gte => (
                    Bound::Included(Term::from_field_u64(field, value)),
                    Bound::Unbounded,
                ),
            };

            let range_query = tantivy::query::RangeQuery::new(lower_bound, upper_bound);

            if *negated {
                Some(Box::new(BooleanQuery::from(vec![(
                    Occur::MustNot,
                    Box::new(range_query) as Box<dyn Query>,
                )])))
            } else {
                Some(Box::new(range_query))
            }
        }
        Expr::FieldExists { field, .. } if is_sql_only_field(field) => None,
        Expr::FieldExists {
            field,
            negated,
        } => {
            // `field:` with no value filters for documents that have any
            // non-null value in the field. Tantivy's `ExistsQuery` requires
            // fast fields, which our TEXT | STORED schema doesn't have, so
            // we match any token in the inverted index via a `.*` regex.
            let tantivy_field = schema.get_field(field).unwrap();
            let regex_query = RegexQuery::from_pattern(".*", tantivy_field)
                .expect("wildcard regex should compile");
            if *negated {
                Some(Box::new(BooleanQuery::from(vec![
                    (Occur::Must, Box::new(AllQuery) as Box<dyn Query>),
                    (
                        Occur::MustNot,
                        Box::new(regex_query) as Box<dyn Query>,
                    ),
                ])))
            } else {
                Some(Box::new(regex_query))
            }
        }
        Expr::And(left, right) => {
            // This handles the following cases:
            // - Left and right expressions have a query term
            // - Only the left expression has a query term
            // - Only the right expression has a query term
            // - Neither left or right expressions have a query term
            let left_query = aql_to_index_query(idx, schema, left);
            let right_query = aql_to_index_query(idx, schema, right);
            if let Some(lq) = left_query {
                if let Some(rq) = right_query {
                    Some(Box::new(BooleanQuery::from(vec![
                        (Occur::Must, lq),
                        (Occur::Must, rq),
                    ])))
                } else {
                    Some(Box::new(BooleanQuery::from(vec![(Occur::Must, lq)])))
                }
            } else if let Some(rq) = right_query {
                Some(Box::new(BooleanQuery::from(vec![(Occur::Must, rq)])))
            } else {
                None
            }
        }
        Expr::Or(left, right) => {
            let left_query = aql_to_index_query(idx, schema, left);
            let right_query = aql_to_index_query(idx, schema, right);
            if let Some(lq) = left_query {
                if let Some(rq) = right_query {
                    Some(Box::new(BooleanQuery::from(vec![
                        (Occur::Should, lq),
                        (Occur::Must, rq),
                    ])))
                } else {
                    Some(Box::new(BooleanQuery::from(vec![(Occur::Should, lq)])))
                }
            } else if let Some(rq) = right_query {
                Some(Box::new(BooleanQuery::from(vec![(Occur::Should, rq)])))
            } else {
                None
            }
        }
    }
}

pub fn expr_to_sql(expr: &Expr) -> Option<String> {
    fn is_allowed(field: &str) -> bool {
        matches!(field, "scheduled" | "deadline" | "closed" | "date")
    }

    match expr {
        Expr::Term {
            field: Some(field),
            value,
            negated,
            ..
        } if is_allowed(field) => {
            let cmp = if *negated { "!=" } else { "=" };
            Some(format!(
                r#"{} {} '{}'"#,
                field,
                cmp,
                value.replace('\'', "''")
            ))
        }
        Expr::Range {
            field,
            op,
            value,
            negated,
        } if is_allowed(field) => {
            let op_str = match op {
                RangeOp::Lt => {
                    if *negated {
                        ">="
                    } else {
                        "<"
                    }
                }
                RangeOp::Lte => {
                    if *negated {
                        ">"
                    } else {
                        "<="
                    }
                }
                RangeOp::Gt => {
                    if *negated {
                        "<="
                    } else {
                        ">"
                    }
                }
                RangeOp::Gte => {
                    if *negated {
                        "<"
                    } else {
                        ">="
                    }
                }
            };
            Some(format!(
                r#"{} {} '{}'"#,
                field,
                op_str,
                value.replace('\'', "''")
            ))
        }
        Expr::And(left, right) => {
            let l = expr_to_sql(left);
            let r = expr_to_sql(right);
            match (l, r) {
                (Some(l), Some(r)) => Some(format!("({} AND {})", l, r)),
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                _ => None,
            }
        }
        Expr::Or(left, right) => {
            let l = expr_to_sql(left);
            let r = expr_to_sql(right);
            match (l, r) {
                (Some(l), Some(r)) => Some(format!("({} OR {})", l, r)),
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                _ => None,
            }
        }
        Expr::FieldExists { field, negated } if is_allowed(field) => {
            let op = if *negated { "IS NULL" } else { "IS NOT NULL" };
            Some(format!("{} {}", field, op))
        }
        _ => None,
    }
}

pub fn query_to_similarity(expr: &Expr) -> Option<String> {
    fn is_allowed(field: &str) -> bool {
        matches!(field, "title" | "body")
    }

    match expr {
        Expr::Term {
            field: Some(field),
            value,
            negated,
            ..
        } if is_allowed(field) => {
            if *negated {
                None
            } else {
                Some(value.to_owned())
            }
        }
        Expr::And(left, right) => {
            let l = query_to_similarity(left);
            let r = query_to_similarity(right);
            match (l, r) {
                (Some(l), Some(r)) => Some(format!("({} {})", l, r)),
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                _ => None,
            }
        }
        Expr::Or(left, right) => {
            let l = query_to_similarity(left);
            let r = query_to_similarity(right);
            match (l, r) {
                (Some(l), Some(r)) => Some(format!("({} {})", l, r)),
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                _ => None,
            }
        }
        // Field-existence checks don't contribute text to the similarity
        // vector — only concrete term values (title/body) do.
        Expr::FieldExists { .. } => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::aql::parse_query;
    use crate::search::fts::schema::note_schema;
    use tantivy::directory::RamDirectory;
    use tantivy::schema::{Document as _, TantivyDocument, Value as _};
    use tantivy::{doc, IndexWriter, ReloadPolicy};

    /// Build an in-memory Tantivy index with the note schema. Used by
    /// tests that need to exercise tokenization via `tokenizer_for_field`.
    fn build_test_index() -> (Schema, Index) {
        let schema = note_schema();
        let directory = RamDirectory::create();
        let idx = Index::open_or_create(directory, schema.clone()).unwrap();
        (schema, idx)
    }

    /// Index a single document with the given title and body so tests
    /// can search against it.
    fn index_doc(idx: &Index, id: &str, title: &str, body: &str) {
        let schema = idx.schema();
        let id_field = schema.get_field("id").unwrap();
        let title_field = schema.get_field("title").unwrap();
        let body_field = schema.get_field("body").unwrap();
        let type_field = schema.get_field("type").unwrap();

        // Delete any existing doc with this id first (upsert)
        let mut writer: IndexWriter = idx.writer(50_000_000).unwrap();
        let id_term = Term::from_field_text(id_field, id);
        writer.delete_term(id_term);

        let category = schema.get_field("category").unwrap();
        let file_name = schema.get_field("file_name").unwrap();
        writer
            .add_document(doc!(
                id_field => id,
                type_field => "note",
                title_field => title,
                category => "test",
                body_field => body,
                file_name => "test.org"
            ))
            .unwrap();
        writer.commit().unwrap();

        // Force the reader to see the new doc
        let reader = idx.reader_builder().reload_policy(ReloadPolicy::Manual).try_into().unwrap();
        drop(reader);
    }

    /// Index a document with an explicit `status` value, for testing
    /// field-exists queries against the status field.
    fn index_doc_with_status(idx: &Index, id: &str, title: &str, body: &str, status_val: &str) {
        let schema = idx.schema();
        let id_field = schema.get_field("id").unwrap();
        let title_field = schema.get_field("title").unwrap();
        let body_field = schema.get_field("body").unwrap();
        let type_field = schema.get_field("type").unwrap();
        let status_field = schema.get_field("status").unwrap();

        let mut writer: IndexWriter = idx.writer(50_000_000).unwrap();
        let id_term = Term::from_field_text(id_field, id);
        writer.delete_term(id_term);

        writer
            .add_document(doc!(
                id_field => id,
                type_field => "note",
                title_field => title,
                body_field => body,
                status_field => status_val,
            ))
            .unwrap();
        writer.commit().unwrap();

        let reader = idx.reader_builder().reload_policy(ReloadPolicy::Manual).try_into().unwrap();
        drop(reader);
    }

    /// Search `idx` for `query_str` (AQL syntax) and return the top doc ids.
    fn search_top_ids(idx: &Index, query_str: &str, limit: usize) -> Vec<String> {
        let schema = idx.schema();
        let expr = parse_query(query_str).unwrap();
        let query = aql_to_index_query(idx, &schema, &expr).expect("query should build");
        execute_query(idx, &*query, limit)
    }

    /// Execute a pre-built query against `idx` and return the top doc ids.
    fn execute_query(idx: &Index, query: &dyn Query, limit: usize) -> Vec<String> {
        use tantivy::collector::TopDocs;
        let schema = idx.schema();
        let reader = idx.reader().unwrap();
        let searcher = reader.searcher();
        let hits = searcher
            .search(query, &TopDocs::with_limit(limit).order_by_score())
            .unwrap();
        hits.iter()
            .map(|(_, addr)| {
                let doc = searcher.doc::<TantivyDocument>(*addr).unwrap();
                let named = doc.to_named_doc(&schema).0;
                named.get("id").unwrap()[0].as_ref().as_str().unwrap().to_string()
            })
            .collect()
    }

    #[test]
    fn test_tokenize_value_lowercases() {
        let (_schema, idx) = build_test_index();
        let body_field = idx.schema().get_field("body").unwrap();
        let title_field = idx.schema().get_field("title").unwrap();

        // Capitalized single word
        let tokens = tokenize_value(&idx, body_field, "Lee");
        assert_eq!(tokens, vec!["lee"]);

        // Uppercase abbreviation
        let tokens = tokenize_value(&idx, title_field, "FMV");
        assert_eq!(tokens, vec!["fmv"]);

        // Multi-word value gets split
        let tokens = tokenize_value(&idx, body_field, "Lee Sedol");
        assert_eq!(tokens, vec!["lee", "sedol"]);

        // Hyphenated text is split by the analyzer
        let tokens = tokenize_value(&idx, body_field, "well-known");
        assert_eq!(tokens, vec!["well", "known"]);
    }

    #[test]
    fn test_tokenize_value_empty_for_punctuation_only() {
        let (_schema, idx) = build_test_index();
        let body_field = idx.schema().get_field("body").unwrap();
        // Pure punctuation yields no tokens (SimpleTokenizer strips it)
        let tokens = tokenize_value(&idx, body_field, "!!!");
        assert!(tokens.is_empty(), "expected empty tokens for punctuation-only input");
    }

    #[test]
    fn test_fuzzy_distance_thresholds() {
        // Short terms get distance 1
        assert_eq!(fuzzy_distance("FMV"), 1);
        assert_eq!(fuzzy_distance("Lee"), 1);
        // 4-char boundary: still distance 1
        assert_eq!(fuzzy_distance("abcd"), 1);
        // Longer terms get distance 2
        assert_eq!(fuzzy_distance("Sedol"), 2);
        assert_eq!(fuzzy_distance("AlphaGo"), 2);
    }

    #[test]
    fn test_aql_to_index_query_with_caps() {
        // Build an expression with capitalized terms that previously
        // failed to match anything because query tokens were not
        // lowercased. After the fix, this should build a real BooleanQuery.
        let (_schema, idx) = build_test_index();
        let expr = parse_query("Lee Sedol").unwrap();
        let query = aql_to_index_query(&idx, &idx.schema(), &expr);
        assert!(
            query.is_some(),
            "query for capitalized terms should build a real query"
        );
        let binding = query.unwrap();
        let bq = binding.as_any().downcast_ref::<BooleanQuery>();
        assert!(bq.is_some(), "top-level query should be a BooleanQuery (And)");
    }

    #[test]
    fn test_build_token_query_fuzzy_short_term() {
        // Short term (≤4 chars) on a fuzzy field (title) should
        // produce a FuzzyTermQuery with distance 1.
        let (_schema, idx) = build_test_index();
        let title_field = idx.schema().get_field("title").unwrap();
        let query = build_token_query(title_field, true, "FMV");
        let fuzzy = query
            .as_any()
            .downcast_ref::<FuzzyTermQuery>();
        assert!(
            fuzzy.is_some(),
            "short term on fuzzy field should be a FuzzyTermQuery"
        );
    }

    #[test]
    fn test_build_token_query_fuzzy_long_term() {
        // Longer term on a fuzzy field should also produce a
        // FuzzyTermQuery (with distance 2 via fuzzy_distance).
        let (_schema, idx) = build_test_index();
        let title_field = idx.schema().get_field("title").unwrap();
        let query = build_token_query(title_field, true, "Sedol");
        let fuzzy = query
            .as_any()
            .downcast_ref::<FuzzyTermQuery>();
        assert!(
            fuzzy.is_some(),
            "long term on fuzzy field should be a FuzzyTermQuery"
        );
    }

    #[test]
    fn test_build_token_query_non_fuzzy_field() {
        // Non-fuzzy field (body) should produce a plain TermQuery
        // regardless of token length.
        let (_schema, idx) = build_test_index();
        let body_field = idx.schema().get_field("body").unwrap();
        let query = build_token_query(body_field, false, "FMV");
        let term_q = query
            .as_any()
            .downcast_ref::<TermQuery>();
        assert!(
            term_q.is_some(),
            "non-fuzzy field should produce a TermQuery, not FuzzyTermQuery"
        );
    }

    #[test]
    fn test_build_token_query_matches_document() {
        // Behavioral check: build_token_query builds a query that actually
        // matches the right document when executed.
        let (_schema, idx) = build_test_index();
        index_doc(&idx, "doc-1", "", "lee sedol alphago");
        let body_field = idx.schema().get_field("body").unwrap();
        let query = build_token_query(body_field, false, "lee");
        let ids = execute_query(&idx, &*query, 10);
        assert!(
            ids.contains(&"doc-1".to_string()),
            "build_token_query should match doc containing the token"
        );
    }

    #[test]
    fn test_build_field_query_empty_tokens_returns_none() {
        // Punctuation-only input produces no tokens, so the query
        // builder should return None.
        let (_schema, idx) = build_test_index();
        let body_field = idx.schema().get_field("body").unwrap();
        let query = build_field_query(&idx, body_field, "body", "!!!", false, false);
        assert!(
            query.is_none(),
            "punctuation-only input should produce no query"
        );
    }

    #[test]
    fn test_build_field_query_single_token_non_negated() {
        // A single token, non-negated, non-phrase should return the
        // per-token query directly (not wrapped in a BooleanQuery).
        let (_schema, idx) = build_test_index();
        let body_field = idx.schema().get_field("body").unwrap();
        let query = build_field_query(&idx, body_field, "body", "lee", false, false);
        assert!(query.is_some(), "single token should produce a query");
        // For body (non-fuzzy), this should be a TermQuery
        let q = query.unwrap();
        let term_q = q.as_any().downcast_ref::<TermQuery>();
        assert!(
            term_q.is_some(),
            "single token on non-fuzzy field should be a TermQuery"
        );
    }

    #[test]
    fn test_build_field_query_multi_token_non_negated() {
        // Multiple tokens, non-negated should return a BooleanQuery(Should)
        // so docs containing any token match.
        let (_schema, idx) = build_test_index();
        let body_field = idx.schema().get_field("body").unwrap();
        let query = build_field_query(&idx, body_field, "body", "lee sedol", false, false);
        assert!(query.is_some(), "multi-token should produce a query");
        let q = query.unwrap();
        let bq = q.as_any().downcast_ref::<BooleanQuery>();
        assert!(
            bq.is_some(),
            "multi-token non-negated should be a BooleanQuery(Should)"
        );
    }

    #[test]
    fn test_build_field_query_phrase() {
        // Phrase query should produce a PhraseQuery with lowercased
        // tokens so it matches indexed text.
        let (_schema, idx) = build_test_index();
        let body_field = idx.schema().get_field("body").unwrap();
        let query = build_field_query(&idx, body_field, "body", "Lee Sedol", true, false);
        assert!(query.is_some(), "phrase should produce a query");
        let q = query.unwrap();
        let pq = q.as_any().downcast_ref::<PhraseQuery>();
        assert!(pq.is_some(), "phrase should produce a PhraseQuery");
    }

    #[test]
    fn test_build_field_query_negated_multi_token() {
        // Negated multi-token should wrap in a BooleanQuery with
        // Must(All) + MustNot(Should of per-token queries).
        let (_schema, idx) = build_test_index();
        let body_field = idx.schema().get_field("body").unwrap();
        let query = build_field_query(&idx, body_field, "body", "lee sedol", false, true);
        assert!(query.is_some(), "negated multi-token should produce a query");
        let q = query.unwrap();
        let bq = q.as_any().downcast_ref::<BooleanQuery>();
        assert!(
            bq.is_some(),
            "negated multi-token should be a BooleanQuery (Must + MustNot)"
        );
    }

    #[test]
    fn test_build_field_query_phrase_matches() {
        // Behavioral check: a phrase query should match docs where
        // the tokens appear in order, not just any doc with the tokens.
        let (_schema, idx) = build_test_index();
        index_doc(&idx, "doc-phrase", "", "lee sedol beats alphago");
        index_doc(&idx, "doc-scattered", "", "sedol then lee later");
        let body_field = idx.schema().get_field("body").unwrap();
        let query = build_field_query(&idx, body_field, "body", "lee sedol", true, false);
        let ids = execute_query(&idx, &*query.unwrap(), 10);
        assert!(
            ids.contains(&"doc-phrase".to_string()),
            "phrase query should match doc with tokens in order"
        );
    }

    #[test]
    fn test_end_to_end_caps_match() {
        // Index a doc with "Lee Sedol" in the body. Before the fix,
        // searching `Lee Sedol` would not match this doc because
        // query term "Lee" did not lowercase to "lee". After the fix,
        // this doc should be the top result.
        let (_schema, idx) = build_test_index();
        index_doc(&idx, "doc-a", "", "Lee Sedol beats AlphaGo at Go");
        index_doc(&idx, "doc-b", "", "unrelated content with no names");

        let results = search_top_ids(&idx, "Lee Sedol", 10);
        assert!(
            !results.is_empty(),
            "search for 'Lee Sedol' should return at least one result"
        );
        assert_eq!(
            results[0], "doc-a",
            "doc containing 'Lee Sedol' should be the top result"
        );
    }

    #[test]
    fn test_end_to_end_abbreviation_match() {
        // FMV is a short uppercase abbreviation. Before the fix, only
        // title-fuzzy matched it (poorly). After the fix, body matches
        // too and the doc with "FMV" in its body ranks first.
        let (_schema, idx) = build_test_index();
        index_doc(&idx, "doc-fmv", "FMV Review", "financial model validation fmv");
        index_doc(&idx, "doc-other", "", "completely unrelated body text");

        let results = search_top_ids(&idx, "FMV", 10);
        assert!(
            !results.is_empty(),
            "search for 'FMV' should return at least one result"
        );
        assert_eq!(
            results[0], "doc-fmv",
            "doc containing 'FMV' should be the top result"
        );
    }

    #[test]
    fn test_phrase_query_uses_analyzer() {
        // A quoted phrase with capitalized tokens should be lowercased
        // by the analyzer before building the PhraseQuery. Otherwise
        // "Lee Sedol" (phrase) would never match a doc containing
        // the lowercased tokens "lee sedol".
        let (_schema, idx) = build_test_index();
        index_doc(&idx, "doc-phrase", "", "lee sedol beats alphago");
        let results = search_top_ids(&idx, "\"Lee Sedol\"", 10);
        assert!(!results.is_empty(), "phrase search for '\"Lee Sedol\"' should match");
        assert_eq!(results[0], "doc-phrase");
    }

    #[test]
    fn test_negated_multitoken_excludes_any_token() {
        // `-lee sedol` parses as And(Neg Term("lee"), Term("sedol"))
        // — two separate AQL terms, so negation applies to "lee"
        // only. Docs with "lee" should be excluded; docs with
        // "sedol" but not "lee" should remain.
        let (_schema, idx) = build_test_index();
        index_doc(&idx, "has-lee", "", "this doc has lee in it");
        index_doc(&idx, "no-lee-has-sedol", "", "this doc only has sedol");
        index_doc(&idx, "neutral", "", "totally unrelated text");

        let results = search_top_ids(&idx, "-lee sedol", 10);
        // Doc "has-lee" should be excluded; docs without "lee"
        // remain (the other term "sedol" is a positive TermQuery).
        assert!(
            !results.iter().any(|id| id == "has-lee"),
            "docs containing 'lee' should be excluded by negation"
        );
    }

    #[test]
    fn test_expr_to_sql_term() {
        let expr = parse_query("scheduled:2025-04-20").unwrap();
        assert_eq!(
            expr_to_sql(&expr),
            Some("scheduled = '2025-04-20'".to_string())
        );

        let expr = parse_query("-closed:2024-01-01").unwrap();
        assert_eq!(
            expr_to_sql(&expr),
            Some("closed != '2024-01-01'".to_string())
        );
    }

    #[test]
    fn test_expr_to_sql_range() {
        let expr = parse_query("date:>2021-10-10").unwrap();
        assert_eq!(expr_to_sql(&expr), Some("date > '2021-10-10'".to_string()));

        let expr = parse_query("-deadline:<=2022-12-31").unwrap();
        assert_eq!(
            expr_to_sql(&expr),
            Some("deadline > '2022-12-31'".to_string())
        );
    }

    #[test]
    fn test_expr_to_sql_drops_unknown() {
        // 'priority' is not an allowed field; should yield None when it's alone.
        let expr = parse_query("priority:high").unwrap();
        assert_eq!(expr_to_sql(&expr), None);

        // If mixed with a valid field, only valid one appears in output.
        let expr = parse_query("priority:high scheduled:2024-12-12").unwrap();
        assert_eq!(
            expr_to_sql(&expr),
            Some("scheduled = '2024-12-12'".to_string())
        );
    }

    #[test]
    fn test_field_exists_builds_regex_query() {
        // `todo:` resolves to status, which is a Tantivy-indexed (non-SQL)
        // field. The query builder should return Some(RegexQuery) that
        // matches docs with any token in the status inverted index.
        let (_schema, idx) = build_test_index();
        let expr = parse_query("todo:").unwrap();
        let query = aql_to_index_query(&idx, &idx.schema(), &expr);
        assert!(query.is_some(), "todo: should build a Tantivy query");
    }

    #[test]
    fn test_field_exists_matches_docs_with_status() {
        // Behavioral check: doc-1 has status="todo", doc-2 has no status.
        // `todo:` (FieldExists on status) should match only doc-1.
        let (_schema, idx) = build_test_index();
        index_doc_with_status(&idx, "doc-with-status", "", "irrelevant body", "todo");
        index_doc(&idx, "doc-no-status", "", "also irrelevant");

        let results = search_top_ids(&idx, "todo:", 10);
        assert!(
            results.contains(&"doc-with-status".to_string()),
            "doc with a status should match todo: — got {:?}",
            results
        );
        assert!(
            !results.contains(&"doc-no-status".to_string()),
            "doc without a status should NOT match todo: — got {:?}",
            results
        );
    }

    #[test]
    fn test_field_exists_negated_matches_docs_without_status() {
        // `-todo:` should match docs that do NOT have a status set.
        let (_schema, idx) = build_test_index();
        index_doc_with_status(&idx, "doc-with-status", "", "body one", "done");
        index_doc(&idx, "doc-no-status", "", "body two");

        let results = search_top_ids(&idx, "-todo:", 10);
        assert!(
            !results.contains(&"doc-with-status".to_string()),
            "doc with status should be excluded by -todo: — got {:?}",
            results
        );
        assert!(
            results.contains(&"doc-no-status".to_string()),
            "doc without status should match -todo: — got {:?}",
            results
        );
    }

    #[test]
    fn test_field_exists_sql_only_returns_none_in_tantivy() {
        // SQL-only fields (scheduled, deadline, closed, date) are handled
        // by the SQL layer; Tantivy should return None for them.
        let (_schema, idx) = build_test_index();
        let expr = parse_query("scheduled:").unwrap();
        assert!(
            aql_to_index_query(&idx, &idx.schema(), &expr).is_none(),
            "SQL-only FieldExists should be dropped from Tantivy"
        );
    }

    #[test]
    fn test_field_exists_sql_for_date_field() {
        // SQL-only FieldExists should emit IS NOT NULL / IS NULL.
        let expr = parse_query("scheduled:").unwrap();
        assert_eq!(
            expr_to_sql(&expr),
            Some("scheduled IS NOT NULL".to_string())
        );

        let expr = parse_query("-closed:").unwrap();
        assert_eq!(
            expr_to_sql(&expr),
            Some("closed IS NULL".to_string())
        );
    }

    #[test]
    fn test_field_exists_non_sql_field_dropped_from_sql() {
        // `todo:` (status) is not a SQL field — Tantivy handles it.
        let expr = parse_query("todo:").unwrap();
        assert_eq!(expr_to_sql(&expr), None);
    }
}
