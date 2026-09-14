#!/usr/bin/env bash
# Install hq to ~/.cargo/bin (assumes it's on the PATH).
# --locked ensures Cargo.lock is used and not regenerated/updated.
set -euo pipefail

cd "$(dirname "$0")/.."

cargo install --path . --locked
