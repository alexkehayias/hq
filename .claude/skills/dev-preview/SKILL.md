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

1. **Run `./bin/run.sh` in the background.** The script handles the Tailscale IP, port selection, Tailwind CSS build, Biome checks, server startup, and readiness polling.

2. **Wait for the readiness line before reporting anything.** The script prints
   ```
   Server ready: http://<host>:<port>
   ```
   only after the server accepts connections. Do not report a URL while the log still shows `Compiling` or only `Starting server on ...` — that URL is not live yet.

   `run.sh` waits up to 5 minutes for the port to open, which covers a cold build of the workspace. If the output ends without `Server ready` anyway, run `./bin/run.sh` again — the build is now warm and the server starts well inside the timeout.

3. **Report the preview URL** from the `Server ready:` line (or from `$HQ_HOST` and `$HQ_PORT`, defaults: `localhost`, `2222`):
   ```
   Preview URL: http://<host>:<port>
   ```
