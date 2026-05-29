#!/usr/bin/env zsh
# Set up a worktree for hq development.
# Creates storage directories, initializes the database and search index,
# and writes .worktree-env with session-level environment variables.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

STORAGE_DIR=".hq-data"
ENV_FILE=".worktree-env"
SETTINGS_FILE=".claude/settings.local.json"

echo "=== Setting up hq worktree ==="
echo "Storage: $STORAGE_DIR"

# Create storage directories
mkdir -p "$STORAGE_DIR"/{db,index,notes,skills,storage,workspace}
echo "  Created directories under $STORAGE_DIR/"

# Write .worktree-env for shell sessions
cat > "$ENV_FILE" <<'EOF'
export HQ_STORAGE_PATH=.hq-data
EOF
echo "  Wrote $ENV_FILE (source it for shell sessions)"

# Write .claude/settings.local.json for Claude Code sessions.
# Merge env vars and DirChanged hook into existing settings.
if [ -f "$SETTINGS_FILE" ] && [ -s "$SETTINGS_FILE" ]; then
  python3 -c "
import json, sys
path = sys.argv[1]
with open(path) as f:
    settings = json.load(f)
settings.setdefault('env', {})
settings['env']['HQ_STORAGE_PATH'] = '.hq-data'
settings.setdefault('hooks', {})
settings['hooks']['DirChanged'] = 'if [ -f .worktree-env ]; then set -a; source .worktree-env; set +a; fi'
with open(path, 'w') as f:
    json.dump(settings, f, indent=2)
    f.write('\n')
" "$SETTINGS_FILE"
else
  cat > "$SETTINGS_FILE" <<'JSON'
{
  "env": {
    "HQ_STORAGE_PATH": ".hq-data"
  },
  "hooks": {
    "DirChanged": "if [ -f .worktree-env ]; then set -a; source .worktree-env; set +a; fi"
  }
}
JSON
fi
echo "  Updated $SETTINGS_FILE with HQ_STORAGE_PATH and DirChanged hook"

# Initialize the database
echo ""
echo "--- Initializing database ---"
cargo run -- init --db 2>&1 | sed 's/^/  /'

# Initialize the search index
echo ""
echo "--- Initializing search index ---"
cargo run -- init --index 2>&1 | sed 's/^/  /'

echo ""
echo "=== Setup complete ==="
echo ""
echo "  Storage path: $ROOT/$STORAGE_DIR"
echo ""
echo "  Next steps:"
echo "    source .worktree-env    # Set env vars for this terminal session"
echo "    ./bin/run.sh            # Start the dev server"
echo ""
