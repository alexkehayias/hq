#!/usr/bin/env zsh
# Create a new git worktree from main, set up the environment, and
# start Claude Code in a tmux session.
#
# Usage: ./bin/develop.sh <branch-name>
#
# This is a thin wrapper around `cargo run -- develop <branch-name>`.
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <branch-name>"
    echo ""
    echo "Creates a new worktree at .claude/worktrees/<branch-name> from main,"
    echo "sets up the dev environment, and starts Claude Code in a tmux session."
    exit 1
fi

cd "$(cd "$(dirname "$0")/.." && pwd)"
cargo run -- develop "$1"
