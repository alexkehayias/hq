#!/usr/bin/env bash
# Install hq to ~/.cargo/bin (assumes it's on the PATH).
# --locked ensures Cargo.lock is used and not regenerated/updated.
set -euo pipefail

cd "$(dirname "$0")/.."

CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [ "$CURRENT_BRANCH" != "main" ]; then
    echo "WARNING: installing from branch '$CURRENT_BRANCH', not 'main'." >&2
    read -r -p "Continue anyway? [y/N] " reply
    case "$reply" in
        [yY] | [yY][eE][sS]) ;;
        *)
            echo "Aborted." >&2
            exit 1
            ;;
    esac
fi

cargo install --path . --locked
