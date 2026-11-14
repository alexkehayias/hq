use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hq_search::{DocumentType, Index, IndexDocument, NewDocument};
use tokio::fs;
use tokio_rusqlite::Connection;

use super::SYNC_LOCK;
use super::parse::{Note, archive_source_id, parse_note};
use super::source::{note_filter, notes};
use crate::channels::notes::{NewEntry, publish_new_note};
use crate::core::fastembed_cache_dir;
use crate::core::orgmode::is_archive_file;

/// This is the primary function to call for indexing. It parses Org files,
/// translates them into index documents, and writes the metadata store plus
/// the requested full-text and vector indexes in one pass.
pub async fn index_all(
    db: &Connection,
    index_path: &str,
    notes_path: &str,
    full_text: bool,
    vector: bool,
    paths: Option<Vec<PathBuf>>,
) -> Result<()> {
    let note_paths = if let Some(path_bufs) = paths {
        note_filter(notes_path, path_bufs).await
    } else {
        notes(notes_path).await
    };

    let mut parsed = Vec::new();
    let mut documents = Vec::new();

    for path in note_paths {
        tracing::debug!("Indexing note: {:?}", path);

        let content = match fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!("Skipping note {:?}: {}", path, e);
                continue;
            }
        };

        // Store the path relative to the notes root so files in
        // subdirectories (notes/, projects/) are locatable.
        let file_name = path
            .strip_prefix(notes_path)
            .unwrap_or(path.as_path())
            .to_str()
            .unwrap_or_default()
            .to_owned();
        let is_archive = is_archive_file(&file_name);
        let source_id = if is_archive {
            archive_source_id(notes_path, &file_name).await
        } else {
            None
        };
        let note = match parse_note(&file_name, source_id.as_deref(), &content) {
            Ok(note) => note,
            Err(e) => {
                tracing::warn!("Skipping note {:?}: {}", path, e);
                continue;
            }
        };

        documents.extend(note_documents(&note, &file_name, is_archive));
        parsed.push((file_name, note));
    }

    let index = Index::open(db.clone(), index_path, fastembed_cache_dir());
    let new_documents = index.upsert(&documents, full_text, vector).await?;
    publish_new_documents(&parsed, new_documents);

    Ok(())
}

/// Re-index a single org file into the metadata store and full-text index.
///
/// Used after task create/update so the changes are immediately queryable
/// instead of waiting for a full `hq index`. The index directory must already
/// exist (created by init / `hq index`); we intentionally do not create it.
pub async fn index_single_file(
    db: &Connection,
    index_path: &str,
    notes_path: &str,
    file_path: PathBuf,
) -> Result<()> {
    let file_name = file_path
        .strip_prefix(notes_path)
        .unwrap_or(file_path.as_path())
        .to_str()
        .unwrap_or_default()
        .to_owned();

    let content = fs::read_to_string(&file_path)
        .await
        .with_context(|| format!("Failed to read file: {:?}", file_path))?;
    let is_archive = is_archive_file(&file_name);
    let source_id = if is_archive {
        archive_source_id(notes_path, &file_name).await
    } else {
        None
    };
    let note = parse_note(&file_name, source_id.as_deref(), &content)?;
    let documents = note_documents(&note, &file_name, is_archive);

    let index = Index::open(db.clone(), index_path, fastembed_cache_dir());
    let new_documents = index.upsert(&documents, true, false).await?;

    publish_new_documents(&[(file_name, note)], new_documents);
    Ok(())
}

/// Remove a task (or any indexed document) from all indexes by its ID.
/// Used after deleting a task from an org file.
pub async fn remove_task_from_indexes(
    db: &Connection,
    index_path: &str,
    task_id: &str,
) -> Result<()> {
    let index = Index::open(db.clone(), index_path, fastembed_cache_dir());
    // Best-effort: a missing index directory leaves nothing to remove, but the
    // metadata cleanup inside `delete` must still run.
    let _ = index.delete(&[task_id.to_string()]).await;
    Ok(())
}

/// Reindex the given note files, whose paths are relative to the notes repo
/// root (as returned by git).
pub async fn reindex_changed_notes(
    db: &Connection,
    index_path: &str,
    notes_path: &str,
    changed: &[String],
    full_text: bool,
    vector: bool,
) -> Result<()> {
    let paths: Vec<PathBuf> = changed
        .iter()
        .map(|file| PathBuf::from(notes_path).join(file))
        .collect();
    index_all(db, index_path, notes_path, full_text, vector, Some(paths)).await
}

/// Reindex specific files while holding the notes-sync lock. Server-side
/// callers that index outside `sync_and_reindex_notes` (e.g. the note status
/// PATCH) must use this so they don't race the periodic sync on the notes
/// working tree.
pub async fn index_files_locked(
    db: &Connection,
    index_path: &str,
    notes_path: &str,
    full_text: bool,
    vector: bool,
    paths: Option<Vec<PathBuf>>,
) -> Result<()> {
    let _guard = SYNC_LOCK.lock().await;
    index_all(db, index_path, notes_path, full_text, vector, paths).await
}

/// Translate a parsed note into the documents the index understands.
fn note_documents(note: &Note, file_name: &str, is_archive: bool) -> Vec<IndexDocument> {
    let mut documents = Vec::new();

    // Archive files share the source note's ID; skip the note-level document
    // so archives don't clobber the source note. Their entries are indexed
    // below.
    if !is_archive {
        documents.push(IndexDocument {
            id: note.id.clone(),
            document_type: DocumentType::Note,
            title: note.title.clone(),
            category: note.category.clone(),
            body: note.body.clone(),
            tags: note.tags.clone(),
            status: None,
            scheduled: None,
            deadline: None,
            closed: None,
            date: None,
            file_name: file_name.to_string(),
            chat_role: None,
            embed: true,
        });
    }

    for meeting in &note.meetings {
        documents.push(IndexDocument {
            id: meeting.id.clone(),
            document_type: DocumentType::Meeting,
            title: meeting.title.clone(),
            category: meeting.category.clone(),
            body: meeting.body.clone(),
            tags: meeting.tags.clone(),
            status: None,
            scheduled: None,
            deadline: None,
            closed: None,
            date: Some(meeting.date.clone()),
            file_name: file_name.to_string(),
            chat_role: None,
            embed: false,
        });
    }

    for heading in &note.headings {
        documents.push(IndexDocument {
            id: heading.id.clone(),
            document_type: DocumentType::Heading,
            title: heading.title.clone(),
            category: heading.category.clone(),
            body: heading.body.clone(),
            tags: heading.tags.clone(),
            status: None,
            scheduled: None,
            deadline: None,
            closed: None,
            date: None,
            file_name: file_name.to_string(),
            chat_role: None,
            embed: false,
        });
    }

    for task in &note.tasks {
        documents.push(IndexDocument {
            id: task.id.clone(),
            document_type: DocumentType::Task,
            title: task.title.clone(),
            category: task.category.clone(),
            body: task.body.clone(),
            tags: task.tags.clone(),
            status: Some(task.status.clone()),
            scheduled: task.scheduled.clone(),
            deadline: task.deadline.clone(),
            closed: task.closed.clone(),
            date: None,
            file_name: file_name.to_string(),
            chat_role: None,
            embed: false,
        });
    }

    documents
}

/// Announce newly indexed entries on the `new-notes` channel so loops can
/// react to them. A no-op unless a server publisher is registered.
fn publish_new_documents(parsed: &[(String, Note)], new_documents: Vec<NewDocument>) {
    if new_documents.is_empty() {
        return;
    }

    let mut by_file: HashMap<&str, Vec<NewEntry>> = HashMap::new();
    for document in &new_documents {
        by_file
            .entry(document.file_name.as_str())
            .or_default()
            .push(NewEntry {
                id: document.id.clone(),
                title: document.title.clone(),
                kind: document.document_type.as_str(),
            });
    }

    for (file_name, note) in parsed {
        if let Some(entries) = by_file.get(file_name.as_str()) {
            publish_new_note(file_name, &note.title, &note.id, entries);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::{async_db, initialize_db};
    use tempfile::TempDir;

    async fn test_db() -> (Connection, TempDir) {
        let dir = TempDir::new().unwrap();
        let vec_db_path = dir.path().to_str().unwrap().to_string();
        let db = async_db(&vec_db_path).await.unwrap();
        db.call(|conn| {
            initialize_db(conn).unwrap();
            Ok::<_, tokio_rusqlite::rusqlite::Error>(())
        })
        .await
        .unwrap();
        (db, dir)
    }

    fn document(id: &str, document_type: DocumentType, title: &str) -> IndexDocument {
        IndexDocument {
            id: id.to_string(),
            document_type,
            title: title.to_string(),
            category: document_type.as_str().to_string(),
            body: String::new(),
            tags: None,
            status: None,
            scheduled: None,
            deadline: None,
            closed: None,
            date: None,
            file_name: "notes/first.org".to_string(),
            chat_role: None,
            embed: false,
        }
    }

    /// `upsert` reports every metadata row as new only on its first insert.
    #[tokio::test]
    async fn test_upsert_reports_new_documents_once() {
        let (db, _db_dir) = test_db().await;
        let index_dir = TempDir::new().unwrap();
        let index = Index::open(db.clone(), index_dir.path(), fastembed_cache_dir());

        let docs = vec![
            document("note-1", DocumentType::Note, "First"),
            document("meeting-1", DocumentType::Meeting, "Meeting"),
            document("heading-1", DocumentType::Heading, "Heading"),
            document("task-1", DocumentType::Task, "Task"),
        ];

        let new_documents = index.upsert(&docs, false, false).await.unwrap();
        assert_eq!(
            new_documents,
            vec![
                NewDocument {
                    id: "note-1".into(),
                    title: "First".into(),
                    document_type: DocumentType::Note,
                    file_name: "notes/first.org".into(),
                },
                NewDocument {
                    id: "meeting-1".into(),
                    title: "Meeting".into(),
                    document_type: DocumentType::Meeting,
                    file_name: "notes/first.org".into(),
                },
                NewDocument {
                    id: "heading-1".into(),
                    title: "Heading".into(),
                    document_type: DocumentType::Heading,
                    file_name: "notes/first.org".into(),
                },
                NewDocument {
                    id: "task-1".into(),
                    title: "Task".into(),
                    document_type: DocumentType::Task,
                    file_name: "notes/first.org".into(),
                },
            ],
            "first index should report every row as new"
        );

        let new_documents = index.upsert(&docs, false, false).await.unwrap();
        assert!(
            new_documents.is_empty(),
            "re-indexing should not report new rows"
        );
    }
}
