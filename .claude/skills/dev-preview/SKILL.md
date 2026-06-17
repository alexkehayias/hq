---
name: dev-preview
description: >-
  Start the hq dev server bound to the machine's Tailscale IP and provide a
  preview URL accessible from any device on the Tailnet. Use this skill whenever
  the user asks for a preview URL, wants to see the app running, needs to check
  the UI in a browser from another device, or wants to share a link to the
  running server. This includes phrases like "preview", "can I see it", "start
  the server", "give me a URL", "show me the app", or "is it running". Do NOT
  use this skill if the user is asking about production deployment, modifying
  server code, or running non-server commands.
---

# dev-preview

Starts the hq development server via `run.sh` bound to the machine's Tailscale IP so it's reachable from any device on the Tailnet.

## Steps

1. **Run `./bin/run.sh`** — that's all that's needed. The script handles the Tailscale IP, port selection, Tailwind CSS build, Biome checks, server startup, and readiness polling.

   Run it in the background so Claude can continue working.

2. **Report the preview URL** — check `$HQ_HOST` and `$HQ_PORT` (defaults: `localhost`, `2222`) and report:
   ```
   Preview URL: http://<host>:<port>
   ```
