pub use hq_search::aql;
pub use hq_search::{DocumentType, recreate_index};

mod chat;
mod index;
mod notes;
mod parse;
mod source;
mod sync;

pub use chat::{delete_chat_session_index, index_all_chat_sessions, index_chat_messages};
pub use index::{
    index_all, index_files_locked, index_single_file, reindex_changed_notes,
    remove_task_from_indexes,
};
pub use notes::search_notes;
pub use sync::sync_and_reindex_notes;

/// Serializes notes-repo sync and server-side indexing so the git working tree
/// and the index writer are not raced.
pub(crate) static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
