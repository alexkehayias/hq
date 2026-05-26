# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`hq` is a personal AI assistant and productivity platform built with Rust. It consists of:
- **Server**: REST API + static file serving via Axum
- **Chat UI**: Progressive web app with offline support (in `web-ui/src/`)
- **CLI**: Terminal interface for commands
- **AI Layer**: Tools, agents, and LLM integrations (Anthropic Claude, OpenAI-compatible)

## Common Commands

### Rust Server & Development
```bash
# Run the server (default: localhost:2222)
cargo run -- serve

# Start dev server with auto-reload (requires watchexec-cli: cargo install --locked watchexec-cli)
./bin/watch.sh

# Run tests
cargo test

# Run a specific test
cargo test api_chat_test

# Initialize (clone notes repo, create indices)
cargo run -- --init

# Index all notes
cargo run -- index --all

# Rebuild indices from scratch
cargo run -- rebuild

# Query notes (default: full-text search)
cargo run -- query --term "search term"
# With vector similarity
cargo run -- query --term "search term" --vector

# Chat session in terminal
cargo run -- chat
```

### Web UI
```bash
cd web-ui
# Lint with Biome (ARM: uses bundled binary; x86: uses npx)
./bin/biome check  # or biome ci .
# Build Tailwind CSS
./bin/tailwindcss -i ./src/input.css -o ./src/output.css -m
```

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

### Search (`src/search/`)
- **Tantivy** for full-text search (FTS) indexing of notes
- **sqlite-vec** for semantic/vector similarity search via Fastembed embeddings
- **AQL (Alex Query Language)** - custom query syntax for fielded terms, phrases, negation, ranges

### AI (`src/ai/`)
- `agents/` - task-specific agents (agenda, email)
- `chat/core.rs` - chat session management, message handling
- `tools/` - AI-callable tools (bash, calendar, email, memory, note_search, tasks, web_search)
- `skills/` - dynamically loaded skill system

### API Server (`src/api/`)
Routes built with Axum:
- `/api/chat` - chat sessions and streaming completions
- `/api/notes` - note CRUD and search
- `/api/email` - Gmail integration
- `/api/calendar` - Google Calendar access
- `/api/kv` - key-value store
- `/api/push` - web push notification subscriptions
- `/api/webhook` - inbound webhook handling
- `/api/metrics` - usage metrics

### Background Jobs (`src/jobs/`)
Periodic jobs spawned on server start using Tokio tasks:
- `DailyAgenda` - generates daily agenda
- `ResearchMeetingAttendees` - prepares meeting context
- `GenerateSessionTitles` - auto-titles chat sessions

### External Integrations (`src/google/`)
- Gmail API via OAuth2
- Google Calendar API
- Custom search API for web search

### CLI (`src/cli/`)
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

## Testing

Integration tests in `tests/` use `tower` and `mockito` for HTTP testing. Tests are database integration tests that share a test database - use `serial_test::serial` annotation to enforce sequential execution when needed.