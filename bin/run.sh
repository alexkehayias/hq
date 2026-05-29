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

# Source worktree env vars if present (set by setup.sh)
if [ -f .worktree-env ]; then
    set -a
    source .worktree-env
    set +a
fi

HOST=localhost
PORT=$(./bin/pick-port.sh)

# Write the port to .worktree-env (preserving existing vars like HQ_STORAGE_PATH)
if ! grep -q '^export HQ_PORT=' .worktree-env 2>/dev/null; then
    echo "export HQ_PORT=$PORT" >> .worktree-env
else
    sed -i '' "s/^export HQ_PORT=.*/export HQ_PORT=$PORT/" .worktree-env
fi
if ! grep -q '^export HQ_HOST=' .worktree-env 2>/dev/null; then
    echo "export HQ_HOST=$HOST" >> .worktree-env
else
    sed -i '' "s/^export HQ_HOST=.*/export HQ_HOST=$HOST/" .worktree-env
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
