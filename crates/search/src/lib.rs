//! Search and indexing engine for notes and chat documents.
//!
//! This crate owns the Tantivy full-text index, the sqlite-vec embeddings, and
//! the canonical document metadata store. It knows nothing about Org mode:
//! callers translate their source data into [`IndexDocument`]s.

pub mod aql;
mod documents;
mod index;
mod query;
mod schema;

pub use aql::{
    DATE_FIELDS, DOCUMENT_TYPES, Expr, FIELD_ALIASES, RangeOp, VALID_FIELDS,
    parse_and_validate_query, parse_query, validate_query,
};
pub use documents::{SearchResult, SearchResults};
pub use index::{DocumentType, Index, IndexDocument, NewDocument, recreate_index};
