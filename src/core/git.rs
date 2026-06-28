use std::path::Path;
use std::sync::Arc;

use tokio::process::Command;
use tokio::sync::Mutex;

/// A client for performing git operations on the notes repository.
///
/// Git operations are serialized through a mutex to prevent concurrent
/// modifications to the same repository. All errors are logged and
/// swallowed — a git failure never blocks the edit that triggered it.
#[derive(Clone)]
pub struct GitClient {
    notes_path: String,
    deploy_key_path: String,
    lock: Arc<Mutex<()>>,
}

impl GitClient {
    pub fn new(notes_path: &str, deploy_key_path: &str) -> Self {
        Self {
            notes_path: notes_path.to_string(),
            deploy_key_path: deploy_key_path.to_string(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    /// Stage and commit a single file change to the notes repository.
    ///
    /// The commit author is set inline (no global git config required).
    /// The commit message is piped via stdin to avoid shell escaping issues.
    pub async fn commit_file(&self, file_path: &Path, message: &str) {
        let _guard = self.lock.lock().await;

        let relative = file_path
            .strip_prefix(&self.notes_path)
            .unwrap_or(file_path);

        let ssh_cmd = format!(
            "GIT_SSH_COMMAND='ssh -i {} -o IdentitiesOnly=yes'",
            self.deploy_key_path
        );

        let shell_cmd = format!(
            "cd {} && {} git add {} && git -c user.name='hq' -c user.email='hq@localhost' commit -F -",
            self.notes_path,
            ssh_cmd,
            relative.display(),
        );

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&shell_cmd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to spawn git commit for {}: {}", relative.display(), e);
                return;
            }
        };

        // Write commit message to stdin and close
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(message.as_bytes()).await;
            // stdin is dropped here, closing the pipe
        }

        let output = match child.wait_with_output().await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("git commit failed for {}: {}", relative.display(), e);
                return;
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("nothing to commit") {
                tracing::warn!(
                    "git commit failed for {}: {}",
                    relative.display(),
                    stderr.trim()
                );
            }
        }
    }
}

/// Build a standardized commit message for note/task changes.
pub fn build_commit_message(action: &str, file_name: &str, details: &str) -> String {
    format!("notes: {} {}\n\n{}", action, file_name, details)
}

/// Clone a repo if it doesn't already exist. Logs errors instead of panicking.
pub async fn maybe_clone_repo(deploy_key_path: &str, url: &str, storage_path: &str) {
    let result = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "GIT_SSH_COMMAND='ssh -i {} -o IdentitiesOnly=yes' git clone {} {}",
            deploy_key_path, url, storage_path
        ))
        .output()
        .await;

    match result {
        Ok(output) => {
            let stdout = std::str::from_utf8(&output.stdout).unwrap_or("");
            if !stdout.is_empty() {
                println!("stdout: {}", stdout);
            }
            let stderr = std::str::from_utf8(&output.stderr).unwrap_or("");
            if !stderr.is_empty() {
                println!("stderr: {}", stderr);
            }
            if !output.status.success() {
                println!("Warning: git clone failed (exit code: {})", output.status);
            }
        }
        Err(e) => {
            println!("Warning: failed to execute git clone: {}", e);
        }
    }
}

/// Pull and reset to origin main branch
pub async fn maybe_pull_and_reset_repo(deploy_key_path: &str, path: &str) {
    let result = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && GIT_SSH_COMMAND='ssh -i {} -o IdentitiesOnly=yes' git fetch origin && git reset --hard origin/main", path, deploy_key_path))
        .output()
        .await;

    match result {
        Ok(output) => {
            let stdout = std::str::from_utf8(&output.stdout).unwrap_or("");
            let stderr = std::str::from_utf8(&output.stderr).unwrap_or("");
            tracing::debug!("stdout: {}\nstderr: {}", stdout, stderr);
        }
        Err(e) => {
            tracing::error!("Failed to pull and reset repo: {}", e);
        }
    }
}

/// Return a list of files that have changed between the last two
/// commits.  Run `maybe_pull_and_reset_repo` before hand if you want
/// to get a list of files that changed on origin.
pub async fn diff_last_commit_files(deploy_key_path: &str, path: &str) -> Vec<String> {
    let result = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "cd {} && GIT_SSH_COMMAND='ssh -i {} -o IdentitiesOnly=yes' git --no-pager diff --name-only HEAD^ HEAD",
            path,
            deploy_key_path
        ))
        .output()
        .await;

    match result {
        Ok(output) => {
            let stdout = std::str::from_utf8(&output.stdout).unwrap_or("");
            let stderr = std::str::from_utf8(&output.stderr).unwrap_or("");
            if !stderr.is_empty() {
                tracing::error!("Git diff failed: {}", stderr);
            }
            if stdout.trim().is_empty() {
                Vec::new()
            } else {
                stdout.trim().split("\n").map(|s| s.to_string()).collect()
            }
        }
        Err(e) => {
            tracing::error!("Git diff failed: {}", e);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_commit_file_creates_commit() {
        let dir = TempDir::new().unwrap();
        let notes_path = dir.path().join("notes");
        fs::create_dir(&notes_path).unwrap();

        // Init a git repo
        Command::new("sh")
            .arg("-c")
            .arg(format!(
                "cd {} && git init && git config user.name test && git config user.email test@test.com",
                notes_path.display()
            ))
            .output()
            .await
            .unwrap();

        let client = GitClient::new(notes_path.to_str().unwrap(), "/dev/null");
        let test_file = notes_path.join("test.org");
        fs::write(&test_file, "content").unwrap();

        client
            .commit_file(&test_file, "notes: create test.org\n\nTest")
            .await;

        // Verify commit exists
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "cd {} && git log --oneline",
                notes_path.display()
            ))
            .output()
            .await
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("notes: create test.org"));
    }

    #[tokio::test]
    async fn test_commit_file_not_a_repo_does_not_panic() {
        let dir = TempDir::new().unwrap();
        let client = GitClient::new(dir.path().to_str().unwrap(), "/dev/null");
        let test_file = dir.path().join("test.org");
        fs::write(&test_file, "content").unwrap();
        // Should not panic or propagate error
        client.commit_file(&test_file, "test").await;
    }

    #[tokio::test]
    async fn test_commit_file_nothing_to_commit_silent() {
        let dir = TempDir::new().unwrap();
        let notes_path = dir.path().join("notes");
        fs::create_dir(&notes_path).unwrap();

        Command::new("sh")
            .arg("-c")
            .arg(format!(
                "cd {} && git init && git config user.name test && git config user.email test@test.com",
                notes_path.display()
            ))
            .output()
            .await
            .unwrap();

        let client = GitClient::new(notes_path.to_str().unwrap(), "/dev/null");
        // File not tracked — nothing to commit
        let test_file = notes_path.join("untracked.org");
        fs::write(&test_file, "content").unwrap();
        // Should not panic or log a warning
        client.commit_file(&test_file, "test").await;
    }

    #[test]
    fn test_build_commit_message() {
        let msg = build_commit_message("update", "test.org", "Changed status to DONE");
        assert_eq!(msg, "notes: update test.org\n\nChanged status to DONE");
    }

    #[test]
    fn test_build_commit_message_create() {
        let msg = build_commit_message(
            "create",
            "2026-06-26--buy-groceries.org",
            "Created task 'Buy groceries'",
        );
        assert_eq!(
            msg,
            "notes: create 2026-06-26--buy-groceries.org\n\nCreated task 'Buy groceries'"
        );
    }
}
