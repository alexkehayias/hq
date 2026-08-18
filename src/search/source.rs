/// Utilities for getting source documents for indexing
use std::path::PathBuf;

/// Recursively walk the `notes/` subdirectory of `path` for `.org` files.
/// Only files under the `notes/` subdirectory are treated as notes; the repo
/// root and the `projects/` subdirectory are excluded.
pub async fn notes(path: &str) -> Vec<PathBuf> {
    let root = format!("{path}/notes");
    let mut result = Vec::new();
    let mut pending = vec![root];

    while let Some(dir) = pending.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            let p = entry.path();
            if meta.is_dir() {
                pending.push(p.to_str().unwrap_or_default().to_string());
            } else {
                let ext = p.extension().unwrap_or_default();
                if ext == "org" {
                    result.push(p);
                }
            }
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
