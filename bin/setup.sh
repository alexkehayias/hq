#!/usr/bin/env zsh
# Set up a worktree for hq development.
# Creates storage directories, initializes the database and search index.
# Note: prefer `cargo run -- develop <name>` which handles everything
# including tmux session creation and env var setup.
#
# Usage: ./bin/setup.sh [target-dir]
#   target-dir: Worktree directory to set up (defaults to current repo root)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-$SCRIPT_DIR}"
cd "$TARGET"

STORAGE_DIR=".hq-data"

echo "=== Setting up hq worktree ==="
echo "Storage: $STORAGE_DIR"

# Create storage directories
mkdir -p "$STORAGE_DIR"/storage
echo "  Created base directories under $STORAGE_DIR/"

# Initialize database and indices
echo ""
echo "--- Running init ---"
cargo run -- init 2>&1 | sed 's/^/  /'

# Load example notes for development
echo ""
echo "--- Loading example data ---"
cargo run -- example-data 2>&1 | sed 's/^/  /'

echo ""
echo "=== Setup complete ==="
echo ""
echo "  Storage path: $PWD/$STORAGE_DIR"
echo ""
echo "  Next steps:"
echo "    export HQ_STORAGE_PATH=.hq-data    # Set env vars for this terminal session"
echo "    ./bin/run.sh                       # Start the dev server"
echo ""
