#!/usr/bin/env zsh
# Set up a worktree for hq development.
# Creates storage directories, initializes the database and search index,
# and writes .worktree-env with session-level environment variables.
#
# Usage: ./bin/setup.sh [target-dir]
#   target-dir: Worktree directory to set up (defaults to current repo root)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-$SCRIPT_DIR}"
cd "$TARGET"

STORAGE_DIR=".hq-data"
ENV_FILE=".worktree-env"
SETTINGS_FILE=".claude/settings.local.json"

echo "=== Setting up hq worktree ==="
echo "Storage: $STORAGE_DIR"

# Create storage directories
mkdir -p "$STORAGE_DIR"/storage
echo "  Created base directories under $STORAGE_DIR/"

# Write .worktree-env for shell sessions
cat > "$ENV_FILE" <<EOF
export HQ_STORAGE_PATH=.hq-data
EOF
echo "  Wrote $ENV_FILE (source it for shell sessions)"

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
echo "    source .worktree-env    # Set env vars for this terminal session"
echo "    ./bin/run.sh            # Start the dev server"
echo ""
