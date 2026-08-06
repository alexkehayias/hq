#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

TAG="${1:-$(git rev-parse --short HEAD)}"
HQ_DOKKU_REMOTE="${HQ_DOKKU_REMOTE:?HQ_DOKKU_REMOTE must be set (e.g. dokku@your-server.com)}"
HQ_DOKKU_APP="${HQ_DOKKU_APP:?HQ_DOKKU_APP must be set}"
IMAGE_NAME="hq:${TAG}"

# 1. Rebuild Tailwind CSS
echo "==> Rebuilding Tailwind CSS..."
cd web-ui
"$REPO_ROOT/bin/tailwindcss" -i ./src/input.css -o ./src/output.css -m
cd "$REPO_ROOT"

# 2. Build the Docker image (cross-compiles hq for linux/amd64)
echo "==> Building Docker image for linux/amd64..."
docker build -t "${IMAGE_NAME}" -t "hq:latest" .

# 3. Stream the image to Dokku via SSH
echo "==> Streaming image to Dokku..."
docker image save "${IMAGE_NAME}" | ssh "${HQ_DOKKU_REMOTE}" git:load-image "${HQ_DOKKU_APP}" "${IMAGE_NAME}"

# 4. Rebuild the app using the loaded image
echo "==> Rebuilding app..."
ssh "${HQ_DOKKU_REMOTE}" ps:rebuild "${HQ_DOKKU_APP}"

# 5. Cleanup
echo "==> Cleaning up..."
docker rmi "${IMAGE_NAME}" 2>/dev/null || true

echo "==> Done! Deployed ${TAG}"
