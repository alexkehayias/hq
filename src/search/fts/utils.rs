use crate::search::fts::schema::note_schema;
use tantivy;
use tantivy::Index;

/// Resets the index by deleting all data and recreating an empty
/// index. Useful when rebuilding from scratch or migrating the schema
/// since there is no way to do that in place with Tantivy.
pub async fn recreate_index(index_path: &str) {
    tokio::fs::remove_dir_all(index_path)
        .await
        .expect("Failed to delete index directory");
    tokio::fs::create_dir(index_path)
        .await
        .expect("Failed to recreate index directory");
    let index_path =
        tantivy::directory::MmapDirectory::open(index_path).expect("Index not found");
    let schema = note_schema();
    Index::open_or_create(index_path, schema.clone()).expect("Unable to open or create index");
}
