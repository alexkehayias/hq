use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use serde::{Deserialize, Serialize};
use tantivy::schema::*;
use tantivy::{Index as TantivyIndex, IndexWriter, Term, doc};
use text_splitter::{ChunkConfig, TextSplitter};
use tiktoken_rs::{CoreBPE, cl100k_base};
use tokio_rusqlite::{Connection, rusqlite};
use zerocopy::IntoBytes;

use crate::schema::document_schema;

/// Serializes Tantivy writers across the process. Tantivy's directory lock is
/// exclusive and a second writer panics with `LockBusy`, so every writer —
/// server and CLI alike — goes through this lock.
static WRITER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The kind of document stored in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentType {
    Note,
    Task,
    Meeting,
    Heading,
    Chat,
}

impl DocumentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DocumentType::Note => "note",
            DocumentType::Task => "task",
            DocumentType::Meeting => "meeting",
            DocumentType::Heading => "heading",
            DocumentType::Chat => "chat",
        }
    }
}

impl FromStr for DocumentType {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "note" => Ok(DocumentType::Note),
            "task" => Ok(DocumentType::Task),
            "meeting" => Ok(DocumentType::Meeting),
            "heading" => Ok(DocumentType::Heading),
            "chat" => Ok(DocumentType::Chat),
            other => Err(anyhow::anyhow!("invalid document type: {other}")),
        }
    }
}

/// A single document to write into the metadata store, the vector index, and
/// the full-text index.
#[derive(Debug, Clone)]
pub struct IndexDocument {
    pub id: String,
    pub document_type: DocumentType,
    pub title: String,
    pub category: String,
    pub body: String,
    pub tags: Option<String>,
    pub status: Option<String>,
    pub scheduled: Option<String>,
    pub deadline: Option<String>,
    pub closed: Option<String>,
    pub date: Option<String>,
    pub file_name: String,
    pub chat_role: Option<String>,
    /// Generate and store a vector embedding for this document's body.
    /// Only note-level documents are embedded.
    pub embed: bool,
}

/// A document whose metadata row did not previously exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDocument {
    pub id: String,
    pub title: String,
    pub document_type: DocumentType,
    pub file_name: String,
}

/// Handle to the search indexes and their metadata store.
pub struct Index {
    pub(crate) db: Connection,
    pub(crate) index_path: PathBuf,
    pub(crate) embedding_cache_dir: PathBuf,
}

impl Index {
    pub fn open(
        db: Connection,
        index_path: impl Into<PathBuf>,
        embedding_cache_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            db,
            index_path: index_path.into(),
            embedding_cache_dir: embedding_cache_dir.into(),
        }
    }

    /// Write documents into the metadata store, and optionally the vector and
    /// full-text indexes. Returns the documents whose metadata rows are new.
    pub async fn upsert(
        &self,
        docs: &[IndexDocument],
        full_text: bool,
        vector: bool,
    ) -> Result<Vec<NewDocument>> {
        let new_documents = self.upsert_metadata(docs).await?;

        if vector {
            self.upsert_embeddings(docs).await?;
        }

        if full_text {
            self.upsert_full_text(docs).await?;
        }

        Ok(new_documents)
    }

    /// Remove documents from every index. Metadata cleanup always runs; a
    /// full-text failure is returned after it so callers that treat the index
    /// as best-effort can ignore it.
    pub async fn delete(&self, ids: &[String]) -> Result<()> {
        let full_text_result = self.delete_full_text(ids).await;
        self.delete_metadata(ids).await?;
        full_text_result
    }

    async fn upsert_metadata(&self, docs: &[IndexDocument]) -> Result<Vec<NewDocument>> {
        let docs = docs.to_vec();
        let new_documents = self
            .db
            .call(move |conn| {
                let mut new_documents = Vec::new();
                for doc in &docs {
                    let is_new = match doc.document_type {
                        DocumentType::Chat => continue,
                        DocumentType::Note => upsert_note(conn, doc)?,
                        DocumentType::Meeting => upsert_meeting(conn, doc)?,
                        DocumentType::Heading => upsert_heading(conn, doc)?,
                        DocumentType::Task => upsert_task(conn, doc)?,
                    };
                    if is_new {
                        new_documents.push(NewDocument {
                            id: doc.id.clone(),
                            title: doc.title.clone(),
                            document_type: doc.document_type,
                            file_name: doc.file_name.clone(),
                        });
                    }
                }
                Ok::<_, rusqlite::Error>(new_documents)
            })
            .await?;
        Ok(new_documents)
    }

    async fn upsert_embeddings(&self, docs: &[IndexDocument]) -> Result<()> {
        let embeddable: Vec<(String, String)> = docs
            .iter()
            .filter(|doc| doc.embed && doc.document_type != DocumentType::Chat)
            .map(|doc| (doc.id.clone(), doc.body.clone()))
            .collect();

        if embeddable.is_empty() {
            return Ok(());
        }

        let cache_dir = self.embedding_cache_dir.clone();
        let embeddings = tokio::task::spawn_blocking(move || {
            let mut model = TextEmbedding::try_new(
                InitOptions::new(EmbeddingModel::BGESmallENV15)
                    .with_show_download_progress(true)
                    .with_cache_dir(cache_dir),
            )
            .expect("Failed to initialize embedding model");
            let tokenizer = cl100k_base().expect("Failed to initialize tokenizer");
            let splitter = TextSplitter::new(ChunkConfig::new(1280).with_sizer(tokenizer));

            embeddable
                .into_iter()
                .map(|(id, body)| {
                    let vectors = generate_embeddings(&mut model, &splitter, &body);
                    (id, vectors)
                })
                .collect::<Vec<(String, Vec<Vec<f32>>)>>()
        })
        .await
        .context("Embedding generation task failed")?;

        self.db
            .call(move |conn| {
                for (id, vectors) in embeddings {
                    store_embeddings(conn, &id, vectors)?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await?;

        Ok(())
    }

    async fn upsert_full_text(&self, docs: &[IndexDocument]) -> Result<()> {
        let index_path = self.index_path.clone();
        let docs = docs.to_vec();
        let _guard = WRITER_LOCK.lock().await;
        tokio::task::spawn_blocking(move || {
            let directory =
                tantivy::directory::MmapDirectory::open(&index_path).expect("Index not found");
            let schema = document_schema();
            let idx = TantivyIndex::open_or_create(directory, schema.clone())
                .expect("Unable to open or create index");
            let mut writer: IndexWriter = idx
                .writer(50_000_000)
                .expect("Index writer failed to initialize");

            for doc in &docs {
                write_full_text_document(&mut writer, &schema, doc)
                    .expect("Updating full text search failed");
            }

            writer
                .commit()
                .expect("Full text search index failed to commit");
        })
        .await
        .context("Full-text indexing task failed")?;
        Ok(())
    }

    async fn delete_full_text(&self, ids: &[String]) -> Result<()> {
        let index_path = self.index_path.clone();
        let ids = ids.to_vec();
        let _guard = WRITER_LOCK.lock().await;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let directory = tantivy::directory::MmapDirectory::open(&index_path)
                .context("Failed to open tantivy index directory")?;
            let schema = document_schema();
            let idx = TantivyIndex::open_or_create(directory, schema.clone())
                .context("Unable to open or create tantivy index")?;
            let mut writer: IndexWriter = idx
                .writer(50_000_000)
                .context("Tantivy index writer failed to initialize")?;
            let id_field = schema
                .get_field("id")
                .context("Failed to get 'id' field from schema")?;

            for id in &ids {
                writer.delete_term(Term::from_field_text(id_field, id));
            }

            writer.commit().context("Tantivy delete failed to commit")?;
            Ok(())
        })
        .await
        .context("Tantivy delete task failed")?
    }

    async fn delete_metadata(&self, ids: &[String]) -> Result<()> {
        let ids = ids.to_vec();
        self.db
            .call(move |conn| {
                for id in &ids {
                    conn.execute("DELETE FROM vec_items WHERE note_meta_id = ?", [id])
                        .ok();
                    conn.execute("DELETE FROM note_meta WHERE id = ?", [id])
                        .ok();
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await?;
        Ok(())
    }
}

/// Delete all indexed data and recreate an empty full-text index.
pub async fn recreate_index(index_path: &str) {
    tokio::fs::remove_dir_all(index_path)
        .await
        .expect("Failed to delete index directory");
    tokio::fs::create_dir(index_path)
        .await
        .expect("Failed to recreate index directory");
    let directory = tantivy::directory::MmapDirectory::open(index_path).expect("Index not found");
    let schema = document_schema();
    TantivyIndex::open_or_create(directory, schema).expect("Unable to open or create index");
}

fn generate_embeddings(
    model: &mut TextEmbedding,
    splitter: &TextSplitter<CoreBPE>,
    body: &str,
) -> Vec<Vec<f32>> {
    splitter
        .chunks(body)
        .flat_map(|chunk| {
            model
                .embed(vec![chunk], None)
                .expect("Failed to generate embeddings")
        })
        .collect()
}

fn store_embeddings(
    db: &mut rusqlite::Connection,
    document_id: &str,
    embeddings: Vec<Vec<f32>>,
) -> rusqlite::Result<()> {
    let mut insert =
        db.prepare("INSERT OR REPLACE INTO vec_items(note_meta_id, embedding) VALUES (?, ?)")?;
    let mut update = db.prepare("UPDATE vec_items set embedding = ? WHERE note_meta_id = ?")?;

    for embedding in embeddings {
        insert
            .execute(rusqlite::params![document_id, embedding.as_bytes()])
            .unwrap_or_else(|_| {
                update
                    .execute(rusqlite::params![embedding.as_bytes(), document_id])
                    .expect("Update failed")
            });
    }

    Ok(())
}

fn write_full_text_document(
    writer: &mut IndexWriter,
    schema: &Schema,
    document: &IndexDocument,
) -> tantivy::Result<()> {
    let id = schema.get_field("id")?;
    let r#type = schema.get_field("type")?;
    let title = schema.get_field("title")?;
    let category = schema.get_field("category")?;
    let body = schema.get_field("body")?;
    let tags = schema.get_field("tags")?;
    let status = schema.get_field("status")?;
    let file_name = schema.get_field("file_name")?;
    let chat_role = schema.get_field("chat_role")?;

    // Delete first to get upsert behavior.
    let term_id = Term::from_field_text(id, &document.id);
    writer.delete_term(term_id);

    let mut doc = doc!(
        id => document.id.as_str(),
        r#type => document.document_type.as_str(),
        title => document.title.as_str(),
        category => document.category.clone(),
        body => document.body.as_str(),
        file_name => document.file_name.as_str(),
    );

    if let Some(tag_list) = &document.tags {
        doc.add_text(tags, tag_list);
    }
    if let Some(status_value) = &document.status {
        doc.add_text(status, status_value);
    }
    if let Some(role) = &document.chat_role {
        doc.add_text(chat_role, role);
    }

    writer.add_document(doc)?;
    Ok(())
}

fn upsert_note(conn: &mut rusqlite::Connection, doc: &IndexDocument) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "INSERT INTO note_meta(id, type, category, file_name, title, tags, body) VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET type = excluded.type, category = excluded.category, \
         file_name = excluded.file_name, title = excluded.title, tags = excluded.tags, \
         body = excluded.body, status = NULL, scheduled = NULL, deadline = NULL, \
         closed = NULL, date = NULL, index_updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         RETURNING (index_updated_at IS NULL) AS is_new",
    )?;
    stmt.query_row(
        rusqlite::params![
            doc.id,
            "note",
            doc.category,
            doc.file_name,
            doc.title,
            doc.tags,
            doc.body
        ],
        |row| row.get(0),
    )
}

fn upsert_meeting(conn: &mut rusqlite::Connection, doc: &IndexDocument) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "INSERT INTO note_meta(id, type, category, file_name, title, tags, body, date) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET type = excluded.type, category = excluded.category, \
         file_name = excluded.file_name, title = excluded.title, tags = excluded.tags, \
         body = excluded.body, date = excluded.date, status = NULL, scheduled = NULL, \
         deadline = NULL, closed = NULL, \
         index_updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         RETURNING (index_updated_at IS NULL) AS is_new",
    )?;
    stmt.query_row(
        rusqlite::params![
            doc.id,
            "meeting",
            doc.category,
            doc.file_name,
            doc.title,
            doc.tags,
            doc.body,
            doc.date
        ],
        |row| row.get(0),
    )
}

fn upsert_heading(conn: &mut rusqlite::Connection, doc: &IndexDocument) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "INSERT INTO note_meta(id, type, category, file_name, title, tags, body) VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET type = excluded.type, category = excluded.category, \
         file_name = excluded.file_name, title = excluded.title, tags = excluded.tags, \
         body = excluded.body, status = NULL, scheduled = NULL, deadline = NULL, \
         closed = NULL, date = NULL, index_updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         RETURNING (index_updated_at IS NULL) AS is_new",
    )?;
    stmt.query_row(
        rusqlite::params![
            doc.id,
            "heading",
            doc.category,
            doc.file_name,
            doc.title,
            doc.tags,
            doc.body
        ],
        |row| row.get(0),
    )
}

fn upsert_task(conn: &mut rusqlite::Connection, doc: &IndexDocument) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "INSERT INTO note_meta(id, type, category, file_name, title, tags, body, status, scheduled, deadline, closed) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET type = excluded.type, category = excluded.category, \
         file_name = excluded.file_name, title = excluded.title, tags = excluded.tags, \
         body = excluded.body, status = excluded.status, scheduled = excluded.scheduled, \
         deadline = excluded.deadline, closed = excluded.closed, date = NULL, \
         index_updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         RETURNING (index_updated_at IS NULL) AS is_new",
    )?;
    stmt.query_row(
        rusqlite::params![
            doc.id,
            "task",
            doc.category,
            doc.file_name,
            doc.title,
            doc.tags,
            doc.body,
            doc.status,
            doc.scheduled,
            doc.deadline,
            doc.closed
        ],
        |row| row.get(0),
    )
}
