#!/usr/bin/env zsh
# Find the next available TCP port starting from a given base port.
# Usage: ./bin/pick-port.sh [base_port]
#   base_port defaults to 2222
# Outputs the first available port to stdout.
set -euo pipefail

BASE="${1:-2222}"

PORT=$BASE
while nc -z 127.0.0.1 "$PORT" 2>/dev/null; do
  PORT=$((PORT + 1))
  if [ "$PORT" -gt 65535 ]; then
    echo "Error: no available ports found above $BASE" >&2
    exit 1
  fi
done

echo "$PORT"
