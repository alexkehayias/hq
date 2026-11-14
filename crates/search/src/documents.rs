use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::schema::{TantivyDocument, Value};
use tantivy::{Document, Index as TantivyIndex, ReloadPolicy};
use tokio_rusqlite::{Connection, rusqlite};
use zerocopy::IntoBytes;

use crate::aql::Expr;
use crate::index::{DocumentType, Index};
use crate::query::{aql_to_index_query, expr_to_sql, query_to_similarity};
use crate::schema::document_schema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentHit {
    pub(crate) id: String,
    pub(crate) document_type: DocumentType,
}

/// A hydrated document result. Serialized by the HTTP API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub r#type: String,
    pub title: String,
    pub category: String,
    pub file_name: String,
    pub tags: Option<String>,
    pub is_task: bool,
    pub task_status: Option<String>,
    pub task_scheduled: Option<String>,
    pub task_deadline: Option<String>,
    pub task_closed: Option<String>,
    pub meeting_date: Option<String>,
    pub body: String,
    #[serde(default)]
    pub chat_role: Option<String>,
    /// For chat results, the session the message belongs to.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// The outcome of a search. Note-like documents are hydrated here; chat
/// message IDs are returned for the caller to hydrate from its own store.
#[derive(Debug, Default)]
pub struct SearchResults {
    pub documents: Vec<SearchResult>,
    pub chat_message_ids: Vec<String>,
}

impl Index {
    /// Search the full-text index, optionally appending vector-similarity
    /// results, and hydrate the note-like documents from the metadata store.
    pub async fn search(
        &self,
        query: &Expr,
        include_similarity: bool,
        truncate: bool,
        limit: usize,
    ) -> Result<SearchResults> {
        // The limit of search hits needs to be high enough here for broad
        // queries like `status:todo deadline:>2025-04-01` otherwise results
        // will be unexpectedly missing.
        let mut hits = full_text_search(&self.index_path, query, 10_000).unwrap_or_default();

        if include_similarity {
            let mut similar = similar_documents(&self.db, &self.embedding_cache_dir, query, limit)
                .await
                .unwrap_or_default();
            hits.append(&mut similar);
            hits = dedupe(hits);
        }

        let mut chat_message_ids = Vec::new();
        let mut document_ids = Vec::new();
        for hit in hits {
            match hit.document_type {
                DocumentType::Chat => chat_message_ids.push(hit.id),
                _ => document_ids.push(hit.id),
            }
        }

        let documents = load_documents(&self.db, &document_ids, query, truncate, limit).await?;
        Ok(SearchResults {
            documents,
            chat_message_ids,
        })
    }
}

fn full_text_search(index_path: &Path, query: &Expr, limit: usize) -> Result<Vec<DocumentHit>> {
    let directory = tantivy::directory::MmapDirectory::open(index_path)?;
    let schema = document_schema();
    let idx = TantivyIndex::open(directory)?;

    let reader = idx
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()?;
    let searcher = reader.searcher();

    // A query can contain only SQL-only fields, in which case there is
    // nothing for Tantivy to match.
    let Some(index_query) = aql_to_index_query(&idx, &schema, query) else {
        return Ok(Vec::new());
    };

    let results = searcher.search(&index_query, &TopDocs::with_limit(limit).order_by_score())?;

    let mut hits = Vec::new();
    for (_score, doc_addr) in results {
        let doc = searcher
            .doc::<TantivyDocument>(doc_addr)?
            .to_named_doc(&schema)
            .0;

        let id = doc
            .get("id")
            .and_then(|values| values.first())
            .and_then(|value| value.as_ref().as_str())
            .map(str::to_string);
        let document_type = doc
            .get("type")
            .and_then(|values| values.first())
            .and_then(|value| value.as_ref().as_str())
            .and_then(|value| value.parse().ok());

        if let (Some(id), Some(document_type)) = (id, document_type) {
            hits.push(DocumentHit { id, document_type });
        }
    }

    Ok(hits)
}

async fn similar_documents(
    db: &Connection,
    embedding_cache_dir: &Path,
    query: &Expr,
    limit: usize,
) -> Result<Vec<DocumentHit>> {
    // Extract the relevant text to use for similarity search from the AQL
    // query. A query made entirely of non-text fields has nothing to embed.
    let Some(similarity_string) = query_to_similarity(query) else {
        return Ok(Vec::new());
    };

    let cache_dir = embedding_cache_dir.to_path_buf();
    let query_vector = tokio::task::spawn_blocking(move || {
        let mut model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15)
                .with_show_download_progress(true)
                .with_cache_dir(cache_dir),
        )
        .expect("Failed to initialize embedding model");
        model
            .embed(vec![similarity_string], None)
            .expect("Failed to generate embeddings")
            .remove(0)
    })
    .await
    .context("Query embedding task failed")?;

    let result = db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT note_meta.id, note_meta.type
                FROM vec_items
                JOIN note_meta ON note_meta_id = note_meta.id
                AND LOWER(note_meta.title) NOT LIKE LOWER('%journal%')
                WHERE embedding MATCH ? AND k = ?
                ORDER BY distance
                LIMIT ?
                "#,
            )?;
            let found = stmt
                .query_map(
                    [query_vector.as_bytes(), limit.as_bytes(), limit.as_bytes()],
                    |row| {
                        let id: String = row.get(0)?;
                        let document_type: String = row.get(1)?;
                        Ok(DocumentHit {
                            id,
                            document_type: document_type.parse().expect("Invalid document type"),
                        })
                    },
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(found)
        })
        .await?;

    Ok(result)
}

fn dedupe(hits: Vec<DocumentHit>) -> Vec<DocumentHit> {
    let mut seen = HashSet::new();
    hits.into_iter()
        .filter(|hit| seen.insert(hit.id.clone()))
        .collect()
}

async fn load_documents(
    db: &Connection,
    ids: &[String],
    query: &Expr,
    truncate: bool,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let ids_json = serde_json::to_string(ids).unwrap();
    let mut where_clauses = vec!["note_meta.id in (SELECT value from json_each(?))".to_string()];
    if let Some(extra_sql) = expr_to_sql(query) {
        where_clauses.push(extra_sql);
    }
    let where_clause = format!("WHERE {}", where_clauses.join(" AND "));

    let sql = format!(
        r#"
        SELECT
          id,
          type,
          category,
          file_name,
          title,
          tags,
          body,
          status,
          scheduled,
          deadline,
          closed,
          date
        FROM note_meta
        {}
        ORDER BY date DESC, deadline DESC, scheduled DESC, closed DESC
        LIMIT {}
        "#,
        where_clause, limit
    );

    let results = db
        .call(move |conn| {
            let mut stmt = conn.prepare(&sql)?;
            let found = stmt
                .query_map([ids_json.as_bytes()], |row| {
                    let id = row.get(0)?;
                    let r#type = row.get(1)?;
                    let category = row.get(2)?;
                    let file_name = row.get(3)?;
                    let mut title: String = row.get(4)?;
                    let tags = row.get(5)?;
                    let mut body: String = row.get(6)?;
                    let task_status: Option<String> = row.get(7)?;
                    let is_task = task_status.is_some();
                    let task_scheduled = row.get(8)?;
                    let task_deadline = row.get(9)?;
                    let task_closed = row.get(10)?;
                    let meeting_date = row.get(11)?;

                    if truncate {
                        title = title.chars().take(140).collect();
                        body = body.chars().take(240).collect();
                    }

                    Ok(SearchResult {
                        id,
                        r#type,
                        category,
                        file_name,
                        title,
                        tags,
                        body,
                        is_task,
                        task_status,
                        task_scheduled,
                        task_deadline,
                        task_closed,
                        meeting_date,
                        chat_role: None,
                        session_id: None,
                    })
                })?
                .collect::<std::result::Result<Vec<SearchResult>, _>>()?;
            Ok::<_, rusqlite::Error>(found)
        })
        .await?;

    Ok(results)
}
