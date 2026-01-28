use std::fs;
use tantivy;
use tantivy::Index;
use tantivy::schema::*;

pub fn note_schema() -> Schema {
    let mut schema_builder = Schema::builder();
    // There is no primary ID concept in tantivy so this needs to be
    // stored as a raw value using the index type `STRING` instead of
    // `TEXT` otherwise deletes won't work and that will result in
    // duplicates when querying.
    // See https://github.com/quickwit-oss/tantivy/blob/main/examples/deleting_updating_documents.rs#L49
    schema_builder.add_text_field("id", STRING | STORED);
    schema_builder.add_text_field("type", TEXT | STORED);
    schema_builder.add_text_field("category", TEXT | STORED);
    schema_builder.add_text_field("title", TEXT | STORED);
    schema_builder.add_text_field("tags", TEXT | STORED);
    schema_builder.add_text_field("status", TEXT | STORED);
    schema_builder.add_text_field("body", TEXT | STORED);
    schema_builder.add_text_field("file_name", TEXT | STORED);
    schema_builder.build()
}

/// Resets the index by deleting all data and recreating an empty
/// index. Useful when rebuilding from scratch or migrating the schema
/// since there is no way to do that in place with Tantivy.
pub fn recreate_index(index_path: &str) {
    fs::remove_dir_all(index_path).expect("Failed to delete index directory");
    fs::create_dir(index_path).expect("Failed to recreate index directory");
    let index_path = tantivy::directory::MmapDirectory::open(index_path).expect("Index not found");
    let schema = note_schema();
    Index::open_or_create(index_path, schema.clone()).expect("Unable to open or create index");
}
