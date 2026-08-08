use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

/// True if `path` is itself a git repository (has a `.git` entry directly in
/// it). Used to avoid git commands walking up to an enclosing repo — e.g. when
/// the notes dir sits inside the hq repo but was never cloned, `cd <notes> &&
/// git add -A` would otherwise stage and push the hq repo itself.
pub fn is_git_repo(path: &str) -> bool {
    Path::new(path).join(".git").exists()
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

/// Non-destructive pull: fetch origin and rebase local commits on top.
///
/// Unlike `maybe_pull_and_reset_repo` (removed), this preserves local
/// uncommitted changes and local commits. On conflict, runs `git rebase --abort`
/// to return the repo to its pre-rebase state and returns an error.
pub async fn maybe_pull_rebase(deploy_key_path: &str, path: &str) -> Result<()> {
    let ssh = format!("GIT_SSH_COMMAND='ssh -i {} -o IdentitiesOnly=yes'", deploy_key_path);

    // Fetch first so we have origin's latest.
    let fetch = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && {} git fetch origin", path, ssh))
        .output()
        .await
        .context("Failed to run git fetch")?;
    if !fetch.status.success() {
        let stderr = String::from_utf8_lossy(&fetch.stderr);
        anyhow::bail!("git fetch failed: {}", stderr.trim());
    }

    // Rebase local commits on top of origin/main.
    let rebase = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && {} git rebase origin/main", path, ssh))
        .output()
        .await
        .context("Failed to run git rebase")?;
    if !rebase.status.success() {
        let stderr = String::from_utf8_lossy(&rebase.stderr);
        // Abort the rebase to return repo to pre-rebase state. Without this,
        // the repo stays in REBASE-i state and every subsequent git command fails.
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("cd {} && git rebase --abort", path))
            .output()
            .await;
        anyhow::bail!("git rebase failed (aborted): {}", stderr.trim());
    }

    Ok(())
}

/// Get the current HEAD commit hash (full SHA). Used to compute what
/// changed across a pull/rebase by capturing HEAD before and after.
pub async fn head_sha(path: &str) -> Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && git rev-parse HEAD", path))
        .output()
        .await
        .context("Failed to run git rev-parse")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git rev-parse HEAD failed: {}", stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// List files that changed between two commits (git diff --name-only
/// <from>..<to>). Returns relative paths as printed by git (repo-root-relative).
pub async fn changed_files_between(path: &str, from: &str, to: &str) -> Result<Vec<String>> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "cd {} && git --no-pager diff --name-only {}..{}",
            path, from, to
        ))
        .output()
        .await
        .context("Failed to run git diff")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .trim()
        .split('\n')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Sync local changes to remote. Steps:
///   1. Capture `head_sha(path)` BEFORE any operations
///   2. git add -A
///   3. If nothing staged (git diff --cached --quiet), skip commit but still
///      do the pull/rebase/push below (to integrate remote changes)
///   4. Otherwise: git -c user.name="hq sync" -c user.email="hq@localhost"
///      commit -m "auto-sync from hq"
///   5. git fetch origin && git rebase origin/main
///      - On non-zero exit (conflict): `git rebase --abort`, return Err.
///        Repo stays clean at pre-rebase HEAD.
///   6. git push origin main
///      - On non-zero exit (remote moved): log warning, continue. Next tick retries.
///   7. Return Ok(changed_files_between(path, &pre_head, "HEAD")) so caller
///      can reindex files touched by the rebase (origin's new contributions +
///      our own rebased commit — reindexing our own writes is idempotent).
///
/// Note: there's a small race window between `update_note`'s `tokio::fs::write`
/// and our `git add -A` that could stage a half-written file. Risk is low
/// (atomic at OS level for small files) and self-heals on the next tick when
/// the complete file gets committed. Not blocking.
pub async fn sync_repo(deploy_key_path: &str, path: &str) -> Result<Vec<String>> {
    let ssh = format!("GIT_SSH_COMMAND='ssh -i {} -o IdentitiesOnly=yes'", deploy_key_path);

    // 1. Capture pre-sync HEAD
    let pre_head = head_sha(path).await?;

    // 2. Stage all changes
    let add = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && git add -A", path))
        .output()
        .await
        .context("Failed to run git add")?;
    if !add.status.success() {
        let stderr = String::from_utf8_lossy(&add.stderr);
        anyhow::bail!("git add failed: {}", stderr.trim());
    }

    // 3. Check if anything is staged (git diff --cached --quiet exits 0 = no changes)
    let cached = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && git diff --cached --quiet", path))
        .output()
        .await
        .context("Failed to check staged changes")?;
    let has_staged = !cached.status.success();

    // 4. Commit if there are staged changes
    if has_staged {
        let commit = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "cd {} && git -c user.name='hq sync' -c user.email='hq@localhost' commit -m 'auto-sync from hq'",
                path
            ))
            .output()
            .await
            .context("Failed to run git commit")?;
        if !commit.status.success() {
            let stderr = String::from_utf8_lossy(&commit.stderr);
            // Reset staging so we don't get stuck retrying the same bad commit
            let _ = Command::new("sh")
                .arg("-c")
                .arg(format!("cd {} && git reset", path))
                .output()
                .await;
            anyhow::bail!("git commit failed: {}", stderr.trim());
        }
    }

    // 5. Fetch + rebase
    let fetch = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && {} git fetch origin", path, ssh))
        .output()
        .await
        .context("Failed to run git fetch")?;
    if !fetch.status.success() {
        let stderr = String::from_utf8_lossy(&fetch.stderr);
        // Don't bail on fetch failure — we can still try to push local commits
        tracing::warn!("git fetch failed (continuing): {}", stderr.trim());
    }

    let rebase = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && {} git rebase origin/main", path, ssh))
        .output()
        .await
        .context("Failed to run git rebase")?;
    if !rebase.status.success() {
        let stderr = String::from_utf8_lossy(&rebase.stderr);
        // Abort the rebase to clean up state. Without this, repo stays in
        // REBASE-i and every subsequent git command fails permanently.
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("cd {} && git rebase --abort", path))
            .output()
            .await;
        anyhow::bail!("git rebase failed (aborted): {}", stderr.trim());
    }

    // 6. Push
    let push = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && {} git push origin main", path, ssh))
        .output()
        .await
        .context("Failed to run git push")?;
    if !push.status.success() {
        let stderr = String::from_utf8_lossy(&push.stderr);
        // Push may fail if remote moved (non-fast-forward). Don't bail —
        // local state is correct; next tick will fetch + rebase + retry.
        tracing::warn!("git push failed (will retry next tick): {}", stderr.trim());
    }

    // 7. Compute files changed by the rebase (origin's contributions + our own)
    let post_head = head_sha(path).await.unwrap_or_else(|_| "HEAD".to_string());
    if pre_head == post_head {
        return Ok(Vec::new());
    }
    let changed = changed_files_between(path, &pre_head, "HEAD")
        .await
        .unwrap_or_default();
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: run a git command in `repo_path`, panicking on failure.
    async fn git(repo_path: &str, args: &str) -> String {
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!("cd {} && {}", repo_path, args))
            .output()
            .await
            .expect("git command failed to spawn");
        assert!(
            output.status.success(),
            "git {} failed: stdout={}, stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    /// Helper: initialize a git repo at `path` with an initial commit.
    async fn init_repo(path: &str) {
        git(path, "git init -b main").await;
        git(
            path,
            "git -c user.name=test -c user.email=test@test commit --allow-empty -m initial",
        )
        .await;
    }

    /// `is_git_repo` is true only when the dir is its own git repo, so git
    /// commands won't walk up to an enclosing repo (e.g. the hq repo itself).
    #[tokio::test]
    async fn test_is_git_repo() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        assert!(!is_git_repo(path), "plain dir should not be a git repo");
        init_repo(path).await;
        assert!(is_git_repo(path), "repo root should be a git repo");
    }

    /// `head_sha` returns a 40-char SHA for a git repo, errors on non-git dir.
    #[tokio::test]
    async fn test_head_sha_returns_commit_hash() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        init_repo(path).await;
        let sha = head_sha(path).await.unwrap();
        assert_eq!(sha.len(), 40, "expected full SHA, got: {sha}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn test_head_sha_errors_on_non_git_dir() {
        let tmp = TempDir::new().unwrap();
        let err = head_sha(tmp.path().to_str().unwrap()).await;
        assert!(err.is_err(), "expected error for non-git directory");
    }

    /// `changed_files_between` lists files that differ between two commits.
    #[tokio::test]
    async fn test_changed_files_between() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        init_repo(path).await;
        let pre_head = head_sha(path).await.unwrap();

        // Add a new file and commit it
        fs::write(format!("{path}/newfile.org"), "hello").unwrap();
        git(path, "git add -A").await;
        git(
            path,
            "git -c user.name=test -c user.email=t@t commit -m add-file",
        )
        .await;

        let changed = changed_files_between(path, &pre_head, "HEAD")
            .await
            .unwrap();
        assert_eq!(changed, vec!["newfile.org".to_string()]);
    }

    /// `sync_repo` with no changes: commits nothing, returns empty changed list.
    #[tokio::test]
    async fn test_sync_repo_no_changes() {
        let tmp = TempDir::new().unwrap();
        // Use a bare "remote" so push has somewhere to go
        let remote = TempDir::new().unwrap();
        git(
            remote.path().to_str().unwrap(),
            "git init -b main --bare",
        )
        .await;
        // Init local repo with a remote pointing at the bare one, and push
        // the initial commit so refs/remotes/origin/main exists (sync_repo's
        // `git rebase origin/main` requires it).
        let path = tmp.path().to_str().unwrap();
        init_repo(path).await;
        git(
            path,
            &format!(
                "git remote add origin {}",
                remote.path().to_str().unwrap()
            ),
        )
        .await;
        git(path, "git push -u origin main").await;

        // sync_repo with no local changes — should not error, returns empty Vec
        let changed = sync_repo("unused-deploy-key", path).await.unwrap();
        assert!(changed.is_empty(), "expected no changes, got: {changed:?}");
    }

    /// `sync_repo` with a local change: commits, pushes, returns changed files.
    #[tokio::test]
    async fn test_sync_repo_commits_and_pushes() {
        let remote = TempDir::new().unwrap();
        git(
            remote.path().to_str().unwrap(),
            "git init -b main --bare",
        )
        .await;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        init_repo(path).await;
        git(
            path,
            &format!(
                "git remote add origin {}",
                remote.path().to_str().unwrap()
            ),
        )
        .await;
        // Push initial commit so refs/remotes/origin/main is populated.
        git(path, "git push -u origin main").await;

        // Make a local change (simulating a note edit)
        fs::write(format!("{path}/note.org"), ":PROPERTIES:\n:ID: abc\n:END:").unwrap();

        // sync_repo should commit + push; returned changed list includes note.org
        let changed = sync_repo("unused-deploy-key", path).await.unwrap();
        assert!(
            changed.contains(&"note.org".to_string()),
            "expected note.org in changed files, got: {changed:?}"
        );

        // Verify the push landed on the remote
        let remote_ls = git(
            remote.path().to_str().unwrap(),
            "git --git-dir=. log --name-only --format=format:",
        )
        .await;
        assert!(
            remote_ls.contains("note.org"),
            "expected note.org on remote, got: {remote_ls}"
        );
    }

    /// `maybe_pull_rebase` is non-destructive: local uncommitted changes survive.
    #[tokio::test]
    async fn test_maybe_pull_rebase_preserves_local_changes() {
        let remote = TempDir::new().unwrap();
        git(
            remote.path().to_str().unwrap(),
            "git init -b main --bare",
        )
        .await;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        init_repo(path).await;
        git(
            path,
            &format!(
                "git remote add origin {}",
                remote.path().to_str().unwrap()
            ),
        )
        .await;
        // Push initial commit so origin/main exists on remote
        git(path, "git push -u origin main").await;

        // Make an uncommitted local change
        fs::write(format!("{path}/uncommitted.org"), "local edit").unwrap();

        // maybe_pull_rebase should succeed (nothing new on remote) and NOT
        // clobber our uncommitted change
        let result = maybe_pull_rebase("unused-deploy-key", path).await;
        // Should succeed (or at worst no-op); the key assertion is that
        // uncommitted.org is still present afterwards.
        let _ = result;
        assert!(
            fs::metadata(format!("{path}/uncommitted.org")).is_ok(),
            "uncommitted file was clobbered by pull/rebase"
        );
    }
}