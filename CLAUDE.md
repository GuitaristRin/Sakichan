# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

- `cargo build` — build the entire workspace (both crates)
- `cargo check` — type-check without codegen (fastest feedback)
- `cargo test` — run all unit tests across workspace
- `cargo test -p sakichan-core` — run core crate tests (all co-located inline in source files)
- `cargo test -p sakichan-core -- <test_name>` — run a specific test by name
- `RUST_LOG=debug cargo run` — run with debug logging (default level is `warn`)

## CLI Usage

All interaction is through flat CLI flags (no subcommands):

```bash
# Session management
sakichan-cli --session create                      # Create new session
sakichan-cli --session list                        # List all sessions
sakichan-cli --session <ID>                        # Show session info
sakichan-cli --session <ID> --delete               # Delete session
sakichan-cli --session suspend                     # Suspend session

# Chat
sakichan-cli --session <ID> --prompt "你好"                         # Text chat
sakichan-cli --session <ID> --prompt "这是什么" --image photo.png    # Image chat
sakichan-cli --session <ID> --prompt "..." --file doc.txt            # File context
sakichan-cli --session <ID> --prompt "..." --dsv4f TRUE              # Force DeepSeek

# Branching
sakichan-cli --session <ID> --prompt "..." --deri 1 --order 2

# Session title
sakichan-cli --session <ID> --summary

# API key management
sakichan-cli --token sk-xxx                       # Set API key
sakichan-cli --token clear                        # Clear API keys
```

Key flags: `--session` (required for chat), `--prompt`, `--image` (repeatable), `--file` (repeatable, text files), `--dsv4f TRUE/FALSE`, `--thinking TRUE/FALSE`, `--delete`, `--summary`, `--token`, `--deri`, `--order`.

## Model Strategy

Default model is **SenseNova Flash-Lite** (multimodal, handles both text and images natively).

| Scenario | --dsv4f FALSE (default) | --dsv4f TRUE |
|----------|------------------------|--------------|
| Text | SenseNova Flash-Lite | DeepSeek V4 Flash |
| Image | SenseNova directly (multimodal) | SenseNova analyse → DeepSeek answer |

- SenseNova Flash-Lite: `model: "sensenova-6.7-flash-lite"`, SSE streaming, multimodal (text+image)
- DeepSeek V4 Flash: `model: "deepseek-v4-flash"`, SSE streaming, optional reasoning (thinking mode via `reasoning_effort` parameter)

## Project Architecture

Two-crate workspace (`Cargo.toml` at root):

### `sakichan-core` (library crate)

All business logic, no CLI dependency. Key modules:

- **`config.rs`** — TOML config from `~/.config/sakichan/config.toml`. Auto-generates defaults with model entries (sensenova-flash-lite, sensenova-u1-fast, deepseek-v4-flash). API keys stored as `ENC_KEYRING:com.sakichan/<name>` references.
- **`models/traits.rs`** — `ChatModel` async trait (with `chat_stream()` → SSE), `ImageGenModel` trait, plus shared types `Message`, `StreamEvent` (Token/ReasoningToken/Done/Error), `ChatOptions`, `FinalResult`.
- **`models/sensenova_flash_lite.rs`** — OpenAI-compatible chat model. Multimodal (text + image_url content blocks). No reasoning_content support.
- **`models/deepseek_v4_flash.rs`** — OpenAI-compatible with reasoning support. Emits `reasoning_content` in SSE delta. Extra params merged at request body top level (e.g. `reasoning_effort`).
- **`models/sensenova_u1_fast.rs`** — Image generation via `/images/generations` endpoint. Independent of chat models.
- **`session/store.rs`** — SQLite persistence (`sessions` + `messages` tables). Messages track `depth` + `message_order` for branching. Schema migration via `PRAGMA table_info` → `ALTER TABLE ADD COLUMN`.
- **`session/context.rs`** — In-memory `VecDeque<Message>` with sliding-window truncation (256K token budget, 4K output reserve). Branch tracking (`current_depth/current_order`). Memory injection and context injection support.
- **`memory/store.rs`** — SQLite-backed long-term memory (`long_term_memories` table).
- **`memory/retrieval.rs`** — CJK-aware keyword extraction → LIKE search. Falls back to most recent memories.
- **`pipeline.rs`** — `PipelineContext::run()` orchestrates full chat flow: append message → persist → retrieve memories → inject → sliding window → call model stream → persist response. `PipelineEvent` enum for streaming callbacks.
- **`error.rs`** — `SakichanError` enum with `thiserror`. `pub type Result<T> = std::result::Result<T, SakichanError>`.

### `sakichan-cli` (binary crate)

CLI frontend (tokio + clap + colored + keyring). Only 3 source files:

- **`main.rs`** — Entry point, all handlers (`handle_send_prompt`, `handle_session_create/list/delete/suspend/summary`), model selection logic, image pipeline (two-stage for --dsv4f TRUE, direct for FALSE), file reading, branching logic.
- **`render.rs`** — Terminal output: `StreamingLine` (green during streaming, overwrite via `\r`), `print_info`, `print_model`, `print_usage`, `print_session_entry`.
- **`keyring.rs`** — OS keychain wrapper via `keyring` crate. `set_api_key`, `get_api_key`, `delete_api_key`, `clear_all_api_keys`, `set_all_api_keys`.

### Data Flow

```
User CLI args → main() → handle_send_prompt()
  1. Resolve session (create/load/reuse)
  2. Apply branching (--deri/--order or auto depth+order)
  3. Select model (SenseNova or DeepSeek based on --dsv4f + images)
  4. Build PipelineContext (load messages from SQLite → SessionContext)
  5. Call pipeline.run() or two-stage image pipeline
     a. Append user message to SessionContext
     b. Persist to SQLite messages table
     c. Retrieve long-term memories → inject as system prompt
     d. Sliding-window truncation on message list
     e. Call ChatModel::chat_stream() → SSE HTTP POST
     f. Stream tokens via PipelineEvent callbacks → StreamingLine output
     g. Persist assistant response to SQLite
  6. Print token usage stats
```

## Key Design Decisions

- **Streaming output**: `StreamingLine` prints tokens inline in green as they arrive, using `\r` to overwrite the current line. No per-token prefix repetition.
- **Branching**: Messages stored with `(depth, order)` coordinates. `get_branch_messages()` filters the active path for model context. `--deri` creates parallel branches at a given depth.
- **Keychain**: API keys stored in OS keychain, referenced in config as `ENC_KEYRING:com.sakichan/<name>`. Resolved at startup; `--token` for management.
- **Schema migration**: `initialize_tables()` uses `CREATE TABLE IF NOT EXISTS` followed by `PRAGMA table_info` → `ALTER TABLE ADD COLUMN` for backward-compatible migrations.

## Configuration

- Config file: `~/.config/sakichan/config.toml`
- API keys: OS keychain (keyring crate), stored as `com.sakichan/sensenova` and `com.sakichan/deepseek`
- Database: `~/.local/share/sakichan/sakichan.db` (SQLite)
- Default config auto-generated with entries for sensenova-flash-lite, deepseek-v4-flash, and sensenova-u1-fast