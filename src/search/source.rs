/// Utilities for getting source documents for indexing
use std::path::PathBuf;

/// Get first level files in the directory, does not follow sub
/// directories.
pub async fn notes(path: &str) -> Vec<PathBuf> {
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return vec![];
    };

    // TODO: make this recursive if there is more than one directory of notes
    let mut result = Vec::new();
    while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        // Skip directories and non org files
        let path = entry.path();
        let ext = path.extension().unwrap_or_default();
        let name = path.file_name().unwrap_or_default();
        if meta.is_file() && ext == "org" && name != "config.org" {
            result.push(entry.path());
        }
    }
    result
}

/// Return a list of notes filtered by file names
pub async fn note_filter(path: &str, file_paths: Vec<PathBuf>) -> Vec<PathBuf> {
    // By using the notes source function we also inherit all the
    // extra filtering and rules for which files are eligible so they
    // don't need to be repeated in multiple places.
    notes(path)
        .await
        .into_iter()
        .filter(|p| file_paths.contains(p))
        .collect()
}
