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
NOTES_REPO_URL="${HQ_NOTES_REPO_URL:-stub}"
NOTES_DEPLOY_KEY="${HQ_NOTES_DEPLOY_KEY_PATH:-stub}"

cat > "$ENV_FILE" <<EOF
export HQ_STORAGE_PATH=.hq-data
export HQ_NOTES_REPO_URL=$NOTES_REPO_URL
export HQ_NOTES_DEPLOY_KEY_PATH=$NOTES_DEPLOY_KEY
EOF
echo "  Wrote $ENV_FILE (source it for shell sessions)"

# Write .claude/settings.local.json for Claude Code sessions.
# Merge env vars and DirChanged hook into existing settings.
NOTES_REPO_URL="${HQ_NOTES_REPO_URL:-stub}"
NOTES_DEPLOY_KEY="${HQ_NOTES_DEPLOY_KEY_PATH:-stub}"

if [ -f "$SETTINGS_FILE" ] && [ -s "$SETTINGS_FILE" ]; then
  echo "  Warning: $SETTINGS_FILE already exists with custom settings."
  echo "           Overwriting with worktree defaults. Back up the file if you"
  echo "           need to preserve existing settings."
fi

cat > "$SETTINGS_FILE" <<JSON
{
  "env": {
    "HQ_STORAGE_PATH": ".hq-data",
    "HQ_NOTES_REPO_URL": "$NOTES_REPO_URL",
    "HQ_NOTES_DEPLOY_KEY_PATH": "$NOTES_DEPLOY_KEY"
  },
  "hooks": {
    "DirChanged": "if [ -f .worktree-env ]; then set -a; source .worktree-env; set +a; fi"
  }
}
JSON
echo "  Updated $SETTINGS_FILE with HQ_STORAGE_PATH and DirChanged hook"

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
