# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`hq` is a personal AI assistant and productivity platform built with Rust. It consists of:
- **Server**: REST API + static file serving via Axum
- **Chat UI**: Progressive web app with offline support (in `web-ui/src/`)
- **CLI**: Terminal interface for commands
- **AI Layer**: Tools, agents, and LLM integrations (Anthropic Claude, OpenAI-compatible)

## Workspace Layout

This is a Cargo workspace. The split keeps the fast-moving CLI and API in small
crates so editing them does not recompile the large library.

| Crate | Path | Contents |
|-------|------|----------|
| `hq` (lib) | `src/` | core, ai, channels, loops, jobs, google, openai, anthropic, notify, eval, org, `types` |
| `hq-bash` | `crates/bash/` | vendored bashkit virtual shell (self-contained) |
| `hq-search` | `crates/search/` | Tantivy full-text index, sqlite-vec embeddings, note metadata store, AQL |
| `hq-api` | `crates/api/` | Axum HTTP server and routes |
| `hq-cli` | `crates/cli/` | CLI dispatch and the `hq` binary |

Dependency direction: `hq` → `hq-bash`, `hq-search`; `hq-api` → `hq`, `hq-bash`;
`hq-cli` → `hq`, `hq-api`, `hq-bash`.

`hq-search` is a document-oriented index engine: it knows nothing about Org
mode. `hq` parses Org notes and translates them into `IndexDocument`s.

Because the library cannot depend on the CLI, the `hq` sandbox builtin is
injected: the CLI registers a factory with `hq::ai::tools::set_hq_builtin_factory`
before dispatch. Shared serialized shapes that the library produces and the API
serves live in `hq::types`, not in `hq-api`.

The root package is the library, so run the binary with `-p hq-cli`
(e.g. `cargo run -p hq-cli -- serve`). `cargo build`/`cargo test` without `-p`
cover the whole workspace.

## Common Commands

### Rust Server & Development
```bash
# Run the server (default: localhost:2222, or next available port)
cargo run -p hq-cli -- serve

# Start dev server with auto-reload (requires watchexec-cli: cargo install --locked watchexec-cli)
./bin/watch.sh

# Start dev server on a dynamic port (avoids conflicts with other worktrees)
./bin/run.sh
# Picks the next available port starting from 2222, or uses $HQ_PORT if set.

# Run tests
cargo test

# Run a specific test
cargo test api_chat_test

# Initialize (clone notes repo, create indices)
cargo run -p hq-cli -- --init

# Index all notes
cargo run -p hq-cli -- index --all

# Rebuild indices from scratch
cargo run -p hq-cli -- rebuild

# Apply pending database migrations
cargo run -p hq-cli -- migrate
# Roll back the most recent migration (or --steps N)
cargo run -p hq-cli -- migrate down
# Show applied and pending migrations
cargo run -p hq-cli -- migrate status
# Generate a new migration module
cargo run -p hq-cli -- migrate new add_user_table

# Query notes (default: full-text search)
cargo run -p hq-cli -- query "search term"
# With vector similarity
cargo run -p hq-cli -- query "search term" --vector

# Chat session in terminal
cargo run -p hq-cli -- chat
```

### Web UI
```bash
# Lint with Biome (vendored binary — always run from web-ui/)
cd web-ui && ../bin/biome check
# Auto-fix formatting and lint issues
cd web-ui && ../bin/biome check --write
# Build Tailwind CSS (vendored binary — always run from web-ui/)
cd web-ui && ../bin/tailwindcss -i ./src/input.css -o ./src/output.css -m
```

#### Service Worker Cache

The PWA service worker (`web-ui/src/service-worker.js`) precaches the app shell
into a versioned cache (`CACHE_NAME`). Whenever you add, remove, rename, or
change a file under `web-ui/src/`, you must update the worker:

1. Add/remove the file in the `urlsToCache` list. `cache.addAll` is
   all-or-nothing — a single 404 aborts the install — so the list must match the
   files on disk exactly. Page directories are listed both as `'/foo/'` (what
   the nav links request) and `'/foo/index.html'`.
2. Bump `CACHE_NAME` (e.g. `-v4` → `-v5`).

The version bump changes the worker's bytes, which is what makes the browser
install the new worker; its `activate` handler then deletes the stale cache.
This is how updates reach installed PWAs without a reinstall.

The worker is registered only over HTTPS (see `register-sw.js`), so the
plain-HTTP dev server never runs it and dev reloads always hit the network. In
production it serves static assets cache-first (instant repeat loads) and HTML
navigations network-first (fresh app shell), falling back to the cache offline.

### Docker
```bash
docker build -t hq:latest .
docker run -p 2222:2222 -d hq:latest
```

## Architecture

### Core Data Layer (`src/core/db.rs`)
- SQLite via `rusqlite` and `tokio-rusqlite`
- Vector search via `sqlite-vec` (384-dimensional embeddings from Fastembed)
- Tables: `note_meta`, `vec_items`, `auth`, `push_subscription`, `chat_message`

#### Database Migrations (`src/core/db/migrations/`)

Schema changes are Rust modules with `up` and optional `down` functions. The
framework records applied versions in the `schema_migrations` table, runs each
migration in its own transaction, and runs `down` in reverse for rollbacks.

- `mod.rs` — the `Migration` type and the `migrate` / `rollback` / `status`
  runners. `validate()` checks that versions are positive, unique, and
  ascending, that names are snake_case, and that `HEAD` is in sync. Every
  runner validates first, so a malformed registry fails before touching the
  database; `hq migrate validate` runs the same check without opening a
  database.
- `build.rs` — scans `migration_*.rs`, parses the version and name from each
  filename, and generates the module declarations and `all()` into `OUT_DIR`.
  There is no registry to edit by hand. A malformed filename, duplicate
  version, or non-snake_case name fails the build.
- `migration_0001_initial_schema.rs` — baseline schema plus idempotent upgrades for
  databases created before the framework existed. Do not add new schema changes
  here.
- `HEAD` — points at the newest migration module. `hq migrate new` updates it,
  and `validate()` keeps it in sync with `all()`. Every migration changes this file,
  so two branches that add migrations conflict on merge instead of silently
  diverging.
- To change the schema, add a new `migration_NNNN_<name>.rs` file with `pub fn up`,
  `pub fn down`, and `pub const DOWN: Option<MigrationFn> = Some(down);` (use
  `None` for irreversible migrations). The version comes from the filename;
  keep `down` a true inverse where possible. `hq migrate new <snake_case_name>`
  generates the file with the next version number and updates `HEAD`.

`initialize_db` runs all pending migrations, so `hq init --db` and tests get the
latest schema.

### Search (`src/search/` + `crates/search/`)
- **`hq-search`** owns the index engine: **Tantivy** full-text search,
  **sqlite-vec** semantic search via Fastembed embeddings, and the `note_meta`
  document store
- **`src/search/`** is the Org-aware façade: parsing, note→document translation,
  chat indexing/hydration, git sync, and result composition
- **AQL (Alex Query Language)** - custom query syntax for fielded terms, phrases, negation, ranges

### AI (`src/ai/`)
- `chat/core.rs` - chat session management, message handling
- `tools/` - AI-callable tools (bash, calendar, email, memory, note_search, tasks, web_search)
- `skills/` - dynamically loaded skill system

### API Server (`crates/api/src/`)
Routes built with Axum:
- `/api/chat` - chat sessions and streaming completions
- `/api/notes` - note CRUD and search
- `/api/email` - Gmail integration
- `/api/calendar` - Google Calendar access
- `/api/kv` - key-value store
- `/api/push` - web push notification subscriptions
- `/api/webhook` - inbound webhook handling
- `/api/metrics` - usage metrics

#### Adding a New API Endpoint

**Module structure** — each route group lives in `crates/api/src/routes/<name>/`:
- `mod.rs` — `mod router; pub use router::router;` (re-exports)
- `router.rs` — handler functions + `pub fn router() -> Router<SharedState>`
- `public.rs` — request/response structs (derive `Deserialize`/`Serialize`)

**Handler pattern:**
```rust
type SharedState = Arc<RwLock<AppState>>;

async fn my_handler(
    State(state): State<SharedState>,
    // Optional: Query(params): Query<MyQuery>,
    // Optional: Json(body): Json<MyBody>,
    // Optional: Path(id): Path<String>,
) -> Result<Json<MyResponse>, crate::api::public::ApiError> {
    // Clone what you need from state inside a read lock, then drop it
    let db = state.read().unwrap().db.clone();

    // Async DB: use db.call(move |conn| { ... }).await
    let result = db.call(move |conn| { /* rusqlite */ }).await?;

    // External API calls: direct .await
    let data = some_async_fn().await?;

    Ok(Json(my_response))
}
```

**Error handling:**
- Return `Result<Json<T>, crate::api::public::ApiError>` for fallible handlers
- `ApiError` converts any `Into<anyhow::Error>` via `?`, logs the error, and returns 500
- For non-500 responses: `Ok((StatusCode::NOT_FOUND, "msg").into_response())`

**Registration (3 places):**
1. In `router.rs`: `pub fn router() -> Router<SharedState> { Router::new().route("/path", get(handler)) }`
2. In `routes/mod.rs`: add `pub mod my_module;` and `.nest("/path", my_module::router())`
3. In `crates/api/src/public.rs`: add `pub mod my_module { pub use crate::routes::my_module::public::*; }`

### Background Jobs (`src/jobs/`)
Periodic jobs spawned on server start using Tokio tasks:
- `GenerateSessionTitles` - auto-titles chat sessions
- `GitSync` - syncs the notes repository

### External Integrations (`src/google/`)
- Gmail API via OAuth2
- Google Calendar API
- Custom search API for web search

### CLI (`crates/cli/src/`)
Subcommands via clap: `init`, `migrate`, `serve`, `index`, `rebuild`, `query`, `chat`, `auth`, `job`

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `HQ_STORAGE_PATH` | Base path for indices, notes, DB (default: `./`) |
| `OPENAI_API_KEY` | API key for OpenAI-compatible LLM calls |
| `HQ_LOCAL_LLM_HOST` | Custom LLM endpoint (default: api.openai.com) |
| `HQ_LOCAL_LLM_MODEL` | Model name (default: gpt-4.1-mini) |
| `HQ_NOTES_REPO_URL` | Git repo URL for notes |
| `HQ_GMAIL_CLIENT_ID/SECRET` | Gmail OAuth credentials |
| `HQ_VAPID_KEY_PATH` | Web push notification keys |

## Worktree Development

This project uses git worktrees for isolated development. Each worktree gets its own storage, database, and server port.

```bash
# One-command: create worktree, set up env, start Claude Code in herdr
cargo run -p hq-cli -- develop <branch-name>

# Or manually:
git worktree add .claude/worktrees/<name> main -b <name>
cd .claude/worktrees/<name>
cargo run -p hq-cli -- develop . --no-init --no-examples
./bin/run.sh
```

Key behaviors:
- **`hq develop`**: Creates a worktree, sets up storage, picks a port, runs init, loads example data, writes `.hq-data/.zshrc` with env vars (`HQ_STORAGE_PATH`, `HQ_PORT`, `HQ_HOST`), creates a herdr workspace with `ZDOTDIR` set, and starts Claude Code in it.
- **Ports**: `hq develop` picks an available port starting from 2222 and sets `$HQ_PORT` in the herdr workspace (via `.zshrc`). `run.sh` requires `$HQ_PORT` to be set.
- **Environment**: `hq develop` writes `.hq-data/.zshrc` that sources your zsh config then sets worktree-specific env vars. No env files are persisted to disk.
- **`.claude/worktrees/`** is gitignored — worktree directories are not committed.
- **Storage**: `.hq-data/` contains subdirs (`db/`, `index/`, `notes/`, etc.).
- **Quick start**: `cargo run -p hq-cli -- develop <branch-name>` creates a worktree from main, runs setup, and launches Claude Code in a herdr workspace.
- **herdr**: The develop command uses [herdr](https://herdr.dev/) as its terminal multiplexer. It starts a background herdr server if one isn't running, creates a workspace for the worktree, pre-spawns Claude Code in it, and attaches. Detach with `Ctrl-B Q` (server keeps running). Reattach later with bare `herdr`. Run `herdr server stop` to end sessions. Each worktree gets its own workspace in the shared default session.
- **Worktrees vs workspaces**: A **worktree** is a Git concept — it's a directory on disk where a branch is checked out (created by `git worktree add`). A **workspace** is herdr's project-level container inside the running server; it holds tabs and panes (real terminals). The develop command pairs them 1:1: the worktree is your source code on disk, and the workspace is where Claude Code actually runs. The root pane's cwd points at the worktree directory, so terminals inside the workspace see your code. Detaching herdr leaves both intact — the worktree stays on disk, and the workspace (with Claude still running) persists in the server until you `herdr server stop`.

## Testing

Integration tests in `tests/` use `tower` and `mockito` for HTTP testing. Tests are database integration tests that share a test database - use `serial_test::serial` annotation to enforce sequential execution when needed.