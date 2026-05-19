use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use anyhow::Result;
use tokio::fs;

fn copy_dir_recursive(src: &Path, dest: &Path) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    let src = src.to_path_buf();
    let dest = dest.to_path_buf();
    Box::pin(async move {
        fs::create_dir_all(&dest).await?;

        let mut entries = fs::read_dir(&src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());

            if src_path.is_dir() {
                copy_dir_recursive(&src_path, &dest_path).await?;
            } else {
                fs::copy(&src_path, &dest_path).await?;
            }
        }

        Ok(())
    })
}

pub fn copy_dir(src: &Path, dest: &Path) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    copy_dir_recursive(src, dest)
}
