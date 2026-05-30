use tokio::process::Command;

/// Clone a repo if it doesn't already exist. Returns Ok(()) on success,
/// or an error message on failure (does not panic).
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
