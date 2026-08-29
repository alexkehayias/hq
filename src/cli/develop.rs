use anyhow::{Context, Result};
use serde_json::Value;
use std::env;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio::fs;

use crate::cli::{example_data, init};

/// Per-worktree Claude settings granting common dev-command permissions so Claude Code
/// doesn't re-prompt for cargo/git/gh/hq/bin commands in each new worktree. Destructive
/// ops stay on the `ask` list. Written to `.claude/settings.local.json`, which is
/// gitignored, so it stays personal and is never committed.
const CLAUDE_SETTINGS_LOCAL: &str = r#"{
  "permissions": {
    "allow": [
      "Bash(cargo build *)",
      "Bash(cargo run *)",
      "Bash(cargo test *)",
      "Bash(cargo check *)",
      "Bash(cargo fmt *)",
      "Bash(cargo clippy *)",
      "Bash(git *)",
      "Bash(gh *)",
      "Bash(hq *)",
      "Bash(./bin/*)",
      "Bash(herdr *)"
    ],
    "ask": [
      "Bash(git reset --hard *)",
      "Bash(git push --force*)",
      "Bash(git branch -D *)",
      "Bash(git clean -f*)",
      "Bash(rm -rf *)"
    ]
  }
}"#;

pub async fn run(
    name: String,
    no_init: bool,
    no_examples: bool,
    base_port: Option<u16>,
) -> Result<()> {
    let worktree_path = format!(".claude/worktrees/{name}");
    let base_port = base_port.unwrap_or(2222);

    println!("=== Setting up hq development worktree ===");
    println!("Branch: {name}");
    println!("Path:   {worktree_path}");

    // Step 1: Create git worktree
    if Path::new(&worktree_path).exists() {
        println!("  Worktree already exists, skipping git worktree add");
    } else {
        println!("  Creating git worktree...");
        let status = Command::new("git")
            .args([
                "worktree", "add",
                &worktree_path,
                "main",
                "-b", &name,
            ])
            .status()
            .context("Failed to run git worktree add")?;
        if !status.success() {
            anyhow::bail!("git worktree add failed");
        }
        println!("  Created git worktree at {worktree_path}");
    }

    // Step 2: Change to worktree directory
    env::set_current_dir(&worktree_path)
        .with_context(|| format!("Failed to change to {worktree_path}"))?;

    // Step 3: Create storage directories
    fs::create_dir_all(".hq-data/storage").await?;
    println!("  Created base directories under .hq-data/");

    // Step 3b: Write per-worktree Claude settings so Claude Code doesn't re-prompt for
    // common dev commands (cargo, git, gh, hq, bin scripts) in this worktree.
    write_claude_settings().await?;

    // Get Tailscale IPv4 address for HQ_HOST, falling back to localhost
    let host = Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".to_string());

    // Step 4: Pick port and write custom zsh config
    let port = pick_port(&host, base_port);
    println!("  Picked port {port}");

    let zsh_config_dir = ".hq-data";
    fs::create_dir_all(zsh_config_dir).await?;
    let zshrc_path = format!("{zsh_config_dir}/.zshrc");
    let zshrc_content = format!(
        "# hq worktree zsh configuration\n\
         # Sources the user's config first, then overrides with worktree env vars.\n\
         \n\
         # Source user's global zsh config (sourced for all zsh invocations)\n\
         if [ -f \"$HOME/.zshenv\" ]; then\n\
         \x20  source \"$HOME/.zshenv\"\n\
         fi\n\
         \n\
         # Source user's interactive zsh config\n\
         if [ -f \"$HOME/.zshrc\" ]; then\n\
         \x20  source \"$HOME/.zshrc\"\n\
         fi\n\
         \n\
         # Override with worktree-specific env vars (set after user config)\n\
         export HQ_STORAGE_PATH=.hq-data\n\
         export HQ_PORT={port}\n\
         export HQ_HOST={host}\n"
    );
    fs::write(&zshrc_path, zshrc_content).await?;
    println!("  Wrote .hq-data/.zshrc with worktree env vars");

    // Step 5: Set env vars for child processes
    // SAFETY: setting HQ_STORAGE_PATH before spawning init/example-data.
    // This is called early in the process, no other threads read this env var.
    unsafe { env::set_var("HQ_STORAGE_PATH", ".hq-data") };

    // Step 6: Run init
    if !no_init {
        println!("\n--- Running init ---");
        init::run(true, true, false, true, true, ".hq-data/db", ".hq-data/index", ".hq-data/notes").await?;
    }

    // Step 7: Load example data
    if !no_examples {
        println!("\n--- Loading example data ---");
        example_data::run(".hq-data/notes", ".hq-data/index", ".hq-data/db").await?;
    }

    // Step 8: Ensure herdr server is running (start a background server if needed)
    println!("\n--- Starting herdr ---");
    ensure_herdr_server()?;

    // Step 9: Find an existing workspace for this worktree, or create a new one.
    // Reusing prevents duplicate workspaces when re-running `hq develop <name>` on an
    // existing worktree (tmux used `-s {name}` which failed on collision; herdr has no
    // such guard, so we match by cwd instead).
    let abs_path = fs::canonicalize(".").await?;
    let worktree_abs = abs_path.to_string_lossy().to_string();

    let (workspace_id, root_pane) = match find_workspace_for_cwd(&worktree_abs)? {
        Some(found) => {
            println!("  Reusing existing herdr workspace for {worktree_abs}");
            found
        }
        None => {
            println!("  Creating herdr workspace for {worktree_abs}...");
            create_herdr_workspace(&worktree_abs, &name)?
        }
    };

    // Step 10: Start Claude Code in the workspace's root pane
    start_claude_in_pane(&root_pane)?;
    println!("  Started Claude Code in root pane {root_pane}");

    // Step 11: Focus our workspace so attach shows it (not some other one)
    focus_workspace(&workspace_id)?;

    // Step 12: Attach to herdr (blocks until detach)
    println!("\n=== Setup complete ===");
    println!("  Attaching to herdr (Ctrl-B Q to detach, server keeps running)...\n");
    let status = Command::new("herdr")
        .status()
        .context("Failed to attach to herdr")?;
    if !status.success() {
        anyhow::bail!("herdr attach failed");
    }

    Ok(())
}

/// Ensure a herdr server is running. If `herdr status server` reports running, do nothing.
/// Otherwise spawn `herdr server` as a detached background process (stdio → /dev/null so it
/// doesn't interfere with our later `herdr` attach) and poll until ready (up to 5s).
fn ensure_herdr_server() -> Result<()> {
    if herdr_server_running()? {
        return Ok(());
    }

    println!("  herdr server not running, starting background server...");
    Command::new("herdr")
        .arg("server")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to spawn herdr server — is herdr installed? https://herdr.dev/docs/install/")?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        if herdr_server_running()? {
            println!("  herdr server is ready");
            return Ok(());
        }
    }
    anyhow::bail!("herdr server did not become ready within 5s")
}

/// Check whether `herdr status server` reports a running server.
fn herdr_server_running() -> Result<bool> {
    let output = Command::new("herdr")
        .args(["status", "server"])
        .output()
        .context("Failed to run `herdr status server` — is herdr installed?")?;
    if !output.status.success() {
        return Ok(false);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.contains("status: running"))
}

/// Search existing herdr workspaces for one whose root pane's cwd matches `worktree_abs`.
/// Returns `(workspace_id, root_pane_id)` if found, so we can reuse instead of duplicating.
/// Matches by pane cwd because `workspace list` does not expose a cwd field at the workspace
/// level — only panes carry one.
fn find_workspace_for_cwd(worktree_abs: &str) -> Result<Option<(String, String)>> {
    let list_output = Command::new("herdr")
        .args(["workspace", "list"])
        .output()
        .context("Failed to run `herdr workspace list`")?;
    if !list_output.status.success() {
        return Ok(None);
    }
    let list_json: Value = serde_json::from_slice(&list_output.stdout)
        .context("Failed to parse `herdr workspace list` output as JSON")?;
    let workspaces = list_json
        .get("result")
        .and_then(|r| r.get("workspaces"))
        .and_then(|w| w.as_array())
        .context("workspace list response missing result.workspaces array")?;

    for ws in workspaces {
        let workspace_id = ws
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .with_context(|| format!("workspace entry missing workspace_id: {ws:?}"))?
            .to_string();

        // For each workspace, fetch its panes and look for one whose cwd matches.
        let pane_output = Command::new("herdr")
            .args(["pane", "list", "--workspace", &workspace_id])
            .output()
            .with_context(|| format!("Failed to run `herdr pane list --workspace {workspace_id}`"))?;
        if !pane_output.status.success() {
            continue;
        }
        let pane_json: Value = match serde_json::from_slice(&pane_output.stdout) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let panes = pane_json
            .get("result")
        .and_then(|r| r.get("panes"))
        .and_then(|p| p.as_array());
        if let Some(panes) = panes {
            for pane in panes {
                if pane.get("cwd").and_then(|c| c.as_str()) == Some(worktree_abs) {
                    let root_pane = pane
                        .get("pane_id")
                    .and_then(|v| v.as_str())
                        .with_context(|| format!("pane entry missing pane_id: {pane:?}"))?
                        .to_string();
                    return Ok(Some((workspace_id, root_pane)));
                }
            }
        }
    }
    Ok(None)
}

/// Create a new herdr workspace for `worktree_abs` with label `name`. Sets ZDOTDIR env var
/// pointing at `.hq-data/` so zsh sources our custom .zshrc (which exports HQ_*). Returns
/// `(workspace_id, root_pane_id)`. Uses `--no-focus` so we don't yank an attached client's
/// view mid-setup; explicit `workspace focus` happens later.
fn create_herdr_workspace(worktree_abs: &str, name: &str) -> Result<(String, String)> {
    let zdotdir = format!("{worktree_abs}/.hq-data");
    let output = Command::new("herdr")
        .args([
            "workspace", "create",
            "--cwd", worktree_abs,
            "--label", name,
            "--env", &format!("ZDOTDIR={zdotdir}"),
            "--no-focus",
        ])
        .output()
        .context("Failed to run `herdr workspace create`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("herdr workspace create failed: {stderr}");
    }
    let json: Value = serde_json::from_slice(&output.stdout)
        .context("Failed to parse `herdr workspace create` output as JSON")?;
    let result = json
        .get("result")
        .context("workspace create response missing result")?;
    let root_pane = result
        .get("root_pane")
    .and_then(|r| r.get("pane_id"))
    .and_then(|v| v.as_str())
        .context("workspace create response missing result.root_pane.pane_id")?
        .to_string();
    let workspace_id = result
        .get("workspace")
    .and_then(|w| w.get("workspace_id"))
    .and_then(|v| v.as_str())
        .context("workspace create response missing result.workspace.workspace_id")?
        .to_string();
    Ok((workspace_id, root_pane))
}

/// Run `claude` in the given pane atomically (text + Enter). Claude Code inherits the
/// pane's zsh env, which includes ZDOTDIR (set on workspace create) → .hq-data/.zshrc → HQ_*.
fn start_claude_in_pane(root_pane: &str) -> Result<()> {
    let status = Command::new("herdr")
        .args(["pane", "run", root_pane, "claude"])
        .status()
        .with_context(|| format!("Failed to run `herdr pane run {root_pane} claude`"))?;
    if !status.success() {
        anyhow::bail!("herdr pane run failed");
    }
    Ok(())
}

/// Focus the given workspace so the subsequent `herdr` attach shows our worktree's
/// workspace, not some other one in the shared default session. Best-effort: logs a warning
/// on failure rather than bailing, since attach still works without explicit focus.
fn focus_workspace(workspace_id: &str) -> Result<()> {
    let status = Command::new("herdr")
        .args(["workspace", "focus", workspace_id])
        .status()
        .with_context(|| format!("Failed to run `herdr workspace focus {workspace_id}`"))?;
    if !status.success() {
        eprintln!("  warning: herdr workspace focus failed (attach may show a different workspace)");
    }
    Ok(())
}

/// Write `.claude/settings.local.json` (gitignored) with the per-worktree dev-command
/// allowlist so Claude Code starts each worktree session without re-prompting.
async fn write_claude_settings() -> Result<()> {
    fs::create_dir_all(".claude").await?;
    fs::write(".claude/settings.local.json", CLAUDE_SETTINGS_LOCAL).await?;
    println!("  Wrote .claude/settings.local.json with dev-command permissions");
    Ok(())
}

fn pick_port(host: &str, base: u16) -> u16 {
    for port in base..=u16::MAX {
        // Probe by binding on the same interface the server will use. A connect() against
        // 127.0.0.1 misses servers bound only to the tailnet IP, returning an already-taken
        // port (AddrInUse later). Binding fails with AddrInUse iff the port is taken here.
        if std::net::TcpListener::bind((host, port)).is_ok() {
            return port;
        }
    }
    panic!("No available TCP ports found starting from {base} on {host}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pick_port_returns_valid_port() {
        // Use a very high base port that's almost certainly free
        let port = pick_port("127.0.0.1", u16::MAX - 10);
        assert!(port >= u16::MAX - 10, "port should be >= base");
        assert!(port <= u16::MAX, "port should be <= 65535");
    }

    #[test]
    fn test_pick_port_default_base() {
        let port = pick_port("127.0.0.1", 2222);
        assert!(port >= 2222, "port should be >= default base");
    }
}
