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

Starts the hq development server via `run.sh` but bound to the machine's Tailscale IP so it's reachable from any device on the Tailnet.

## Steps

1. **Get the Tailscale IP** — run `tailscale ip -4` to get the machine's Tailscale IP (e.g., `100.x.x.x`).

2. **Determine the port** — check `$HQ_PORT` first. If not set, fall back to port `2222`.

3. **Start the server using `run.sh`** with the Tailscale IP as the host:
   ```bash
   HOST=<tailscale-ip> HQ_PORT=<port> ./bin/run.sh
   ```
   Run this from the project root (`/Users/ender/Projects/hq`). The `HOST` env var is picked up by `run.sh` (which defaults to `localhost` when unset). The `run.sh` script handles building Tailwind CSS, running Biome checks, starting the server, and waiting for readiness.

   Run this in the background so Claude can continue working.

4. **Report the preview URL** to the user as a clickable markdown link:
   ```
   Preview URL: http://<tailscale-ip>:<port>
   ```

## Notes

- The Tailscale IP changes if the machine reconnects to Tailscale — always fetch it fresh rather than caching.
- `run.sh` already handles: Tailwind CSS build, Biome lint check, server readiness polling, and Chrome tab reload (via osascript). The skill delegates to it rather than duplicating this logic.
- The server runs in the foreground of its background task (via `run.sh`'s `wait $PID`). Run the whole thing with `&` so Claude can continue working.
