use anyhow::{Context, Result};
use std::env;
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::fs;

use crate::cli::{example_data, init};

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
    fs::create_dir_all(".hq-data/storage")?;
    println!("  Created base directories under .hq-data/");

    // Step 4: Pick port and write custom zsh config
    let port = pick_port(base_port);
    println!("  Picked port {port}");

    let zsh_config_dir = ".hq-data";
    fs::create_dir_all(zsh_config_dir)?;
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
         export HQ_HOST=localhost\n"
    );
    fs::write(&zshrc_path, zshrc_content)?;
    println!("  Wrote .hq-data/.zshrc with worktree env vars");

    // Step 5: Set env vars for child processes
    // SAFETY: setting HQ_STORAGE_PATH before spawning init/example-data.
    // This is called early in the process, no other threads read this env var.
    unsafe { env::set_var("HQ_STORAGE_PATH", ".hq-data") };

    // Step 6: Run init
    if !no_init {
        println!("\n--- Running init ---");
        init::run(true, true, true, true, true, ".hq-data/db", ".hq-data/index", ".hq-data/notes").await?;
    }

    // Step 7: Load example data
    if !no_examples {
        println!("\n--- Loading example data ---");
        example_data::run(".hq-data/notes", ".hq-data/index", ".hq-data/db").await?;
    }

    // Step 8: Create tmux session with ZDOTDIR pointing to custom zsh config
    println!("\n--- Creating tmux session ---");
    let abs_path = fs::canonicalize(".")?;
    let tmux_path = abs_path.to_string_lossy().to_string();

    let status = Command::new("tmux")
        .args([
            "new-session", "-d", "-s", &name, "-c", &tmux_path,
            "-e", &format!("ZDOTDIR={tmux_path}/.hq-data"),
        ])
        .status()
        .context("Failed to create tmux session")?;
    if !status.success() {
        anyhow::bail!("tmux new-session failed");
    }
    println!("  Created tmux session with worktree environment");

    // Step 9: Start Claude Code with worktree flag
    let status = Command::new("tmux")
        .args(["send-keys", "-t", &name, "claude", "Space", "--worktree", "Space", &name, "Enter"])
        .status()
        .context("Failed to start Claude Code in tmux")?;
    if !status.success() {
        anyhow::bail!("tmux send-keys failed");
    }
    println!("  Started Claude Code in tmux session '{name}'");

    // Step 10: Attach to the tmux session
    println!("\n=== Setup complete ===");
    println!("  Attaching to tmux session '{name}'...");
    println!("  (Use Ctrl-b d to detach, or exit Claude Code to return)\n");

    let status = Command::new("tmux")
        .args(["attach", "-t", &name])
        .status()
        .context("Failed to attach to tmux session")?;
    if !status.success() {
        anyhow::bail!("tmux attach failed");
    }

    Ok(())
}

fn pick_port(base: u16) -> u16 {
    for port in base..=u16::MAX {
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            return port;
        }
    }
    panic!("No available TCP ports found starting from {base}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pick_port_returns_valid_port() {
        // Use a very high base port that's almost certainly free
        let port = pick_port(u16::MAX - 10);
        assert!(port >= u16::MAX - 10, "port should be >= base");
        assert!(port <= u16::MAX, "port should be <= 65535");
    }

    #[test]
    fn test_pick_port_default_base() {
        let port = pick_port(2222);
        assert!(port >= 2222, "port should be >= default base");
    }
}
