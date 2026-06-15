#!/usr/bin/env zsh
set -euo pipefail

cd ./web-ui

ARCH=$(uname -m)
if [ "$ARCH" = "arm64" ]; then
    ../bin/tailwindcss -i ./src/input.css -o ./src/output.css -m
    ../bin/biome check
else
    pnpx tailwindcss@3.4.11 -i ./src/input.css -o ./src/output.css -m
    pnpx biome check
fi

cd ..

HOST="${HOST:-localhost}"
PORT="${HQ_PORT:?HQ_PORT must be set (hq develop sets this automatically)}"

RUST_BACKTRACE=1 cargo run -- serve --host "$HOST" --port "$PORT" &
PID=$!

echo "Starting server on http://${HOST}:${PORT}"

cleanup() {
    kill $PID 2>/dev/null || true
    exit
}

trap cleanup EXIT INT TERM

TIMEOUT=30
start=$(date +%s)
while ! nc -z "${HOST}" "${PORT}"; do
    (( $(date +%s) - start > TIMEOUT )) && { kill $PID 2>/dev/null || true; exit 1; }
    sleep 0.5
done

echo "Server ready: http://${HOST}:${PORT}"

# Reload the active Chrome browser tab (macOS only)
if [ "$(uname)" = "Darwin" ]; then
    osascript ./bin/reloadChromeTab.scptd
fi

wait $PID
