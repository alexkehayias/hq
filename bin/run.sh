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

HOST=localhost
if [ -z "${HQ_PORT:-}" ]; then
    PORT=2222
    while nc -z 127.0.0.1 "$PORT" 2>/dev/null; do
        PORT=$((PORT + 1))
        if [ "$PORT" -gt 65535 ]; then
            echo "Error: no available ports found" >&2
            exit 1
        fi
    done
else
    PORT="$HQ_PORT"
fi

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

# Reload the active Chrome browser tab
osascript ./bin/reloadChromeTab.scptd

wait $PID
