use tantivy;
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
