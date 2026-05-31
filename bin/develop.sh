#!/usr/bin/env zsh
# Create a new git worktree from main, set up the environment, and
# start Claude Code in a tmux session.
#
# Usage: ./bin/new-worktree.sh <branch-name>
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <branch-name>"
    echo ""
    echo "Creates a new worktree at .claude/worktrees/<branch-name> from main,"
    echo "sets up the dev environment, and starts Claude Code in a tmux session."
    exit 1
fi

BRANCH="$1"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKTREE_PATH="$ROOT/.claude/worktrees/$BRANCH"

echo "=== Creating worktree for branch: $BRANCH ==="

# Create the worktree from main
git worktree add "$WORKTREE_PATH" main -b "$BRANCH"
echo "  Created worktree at $WORKTREE_PATH"

# Change to the worktree directory
cd "$WORKTREE_PATH"

# Run the setup script
./bin/setup.sh

echo ""
echo "=== Starting tmux session ==="

# Start a tmux session in the worktree directory with Claude Code
tmux new-session -d -s "$BRANCH" -c "$WORKTREE_PATH"
tmux send-keys -t "$BRANCH" "claude" Enter

echo "  Started tmux session: $BRANCH"
echo "  Attach with: tmux attach -t $BRANCH"
echo ""
