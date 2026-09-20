#!/usr/bin/env bash
# Point git at the committed hooks in .githooks/.
set -euo pipefail

cd "$(dirname "$0")/.."

git config core.hooksPath .githooks
echo "Git hooks enabled (core.hooksPath -> .githooks)."
