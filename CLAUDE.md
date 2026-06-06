# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

- `cargo build` — build the entire workspace (all three crates)
- `cargo check` — type-check without codegen (fastest feedback)
- `cargo test` — run all unit tests across workspace
- `cargo test -p sakichan-core` — run core crate tests (all co-located inline in source files)
- `cargo test -p sakichan-core -- <test_name>` — run a specific test by name
- `cargo build -p sakichan-gtk` — build the GTK4 GUI binary only (requires gtk4 dev libs)
- `RUST_LOG=debug cargo run --bin sakichan-cli` — run CLI with debug logging (default level is `warn`)

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

# Compress/summarize conversation
sakichan-cli --session <ID> --compressconv

# API key management
sakichan-cli --token sk-xxx                       # Set API key
sakichan-cli --token clear                        # Clear API keys
```

Key flags: `--session` (required for chat), `--prompt`, `--image` (repeatable), `--file` (repeatable, text files), `--dsv4f TRUE/FALSE`, `--thinking TRUE/FALSE`, `--delete`, `--summary`, `--compressconv`, `--token`, `--deri`, `--order`.

## Model Strategy

Default model is **SenseNova Flash-Lite** (multimodal, handles both text and images natively).

| Scenario | --dsv4f FALSE (default) | --dsv4f TRUE |
|----------|------------------------|--------------|
| Text | SenseNova Flash-Lite | DeepSeek V4 Flash |
| Image | SenseNova directly (multimodal) | SenseNova analyse → DeepSeek answer |

- SenseNova Flash-Lite: `model: "sensenova-6.7-flash-lite"`, SSE streaming, multimodal (text+image), 120s timeout
- DeepSeek V4 Flash: `model: "deepseek-v4-flash"`, SSE streaming, optional reasoning (thinking mode via `reasoning_effort` parameter), 180s timeout

## Project Architecture

Three-crate workspace (`Cargo.toml` at root, crates under `crates/`):

### `sakichan-core` (library crate)

All business logic, no UI dependency. Key modules:

- **`config.rs`** — TOML config from `~/.config/sakichan/config.toml`. Auto-generates defaults with model entries (sensenova-flash-lite, sensenova-u1-fast, deepseek-v4-flash). API keys stored as `ENC_KEYRING:com.sakichan/<name>` references; actual keyring resolution is done by the CLI/GTK layer at startup.
- **`models/traits.rs`** — `ChatModel` async trait (with `chat_stream()` → SSE), `ImageGenModel` trait, plus shared types: `Message` (with `images: Vec<String>` skipped in serde, serialized to `image_url` content blocks via `to_api_value()`), `StreamEvent` (Token/ReasoningToken/ToolCall/Done/Error), `ChatOptions`, `FinalResult`, `UsageInfo`.
- **`models/sensenova_flash_lite.rs`** — OpenAI-compatible chat model. Multimodal (text + image_url content blocks). No `reasoning_content` in SSE. Passes `stream_options: {include_usage: true}`.
- **`models/deepseek_v4_flash.rs`** — OpenAI-compatible with reasoning support. Emits `reasoning_content` in SSE delta → `StreamEvent::ReasoningToken`. Extra params (`self.extra_params` + `options.extra_params`) merged at request body top level. No `stream_options` field.
- **`models/sensenova_u1_fast.rs`** — Image generation via `/images/generations` endpoint. Independent of chat models.
- **`session/store.rs`** — SQLite persistence (`sessions` + `messages` + `conversation_summaries` tables). Messages track `depth` + `message_order` for branching. `get_branch_messages()` includes all messages at depths < active_depth, plus only order-matching messages at active_depth. `get_branch_messages_with_coords()` returns `(Message, depth, order)` tuples. Schema migration via `PRAGMA table_info` → `ALTER TABLE ADD COLUMN`.
- **`session/context.rs`** — In-memory `VecDeque<Message>` with sliding-window truncation (256K token budget, 4K output reserve → `(256_000 - 4_000) * 4 = 1_004_000` char budget). Branch tracking (`current_depth/current_order`). `inject_memory_prompt()` and `inject_context_prompt()` for system-level injection (deduplicated by `name` field on replacement).
- **`memory/store.rs`** — SQLite-backed long-term memory (`long_term_memories` table). `search()` uses LIKE per keyword, updates `accessed_at` on hits.
- **`memory/retrieval.rs`** — CJK-aware keyword extraction (bigrams + trigrams for CJK runs, ≥2-char ASCII words minus stop list). Deduplicates results by id; falls back to `list(top_k)` if no keyword matches.
- **`pipeline.rs`** — `PipelineContext::run()` orchestrates full chat flow. `run_image_pipeline()` is the two-stage path (SenseNova analyse → inject description as context prompt → text model). `generate_session_title()` calls model with `max_tokens: 100, temperature: 0.3, reasoning_effort: "none"`. `PipelineEvent` enum: Token/ReasoningToken/MemoriesInjected/NoMemoriesFound/AnalyseImage/Done/Error.
- **`error.rs`** — `SakichanError` enum with `thiserror`. `pub type Result<T> = std::result::Result<T, SakichanError>`.

### `sakichan-cli` (binary crate)

CLI frontend (tokio + clap + colored + keyring). Three source files:

- **`main.rs`** — `Cli` struct (clap derive) with all flags. `AppContext { config, session_store, memory_store }`. Handlers: `handle_send_prompt`, `handle_session_create/list/delete/suspend/summary/info`, `handle_compress_conv`. `select_text_model()` and `build_pipeline_context()` helpers. `load_images()` reads bytes → guesses MIME by extension → base64 data URI. `read_files()` formats as `[文件: <path>]\n<content>` blocks. Keyring values resolved via `resolve_keyring_value()` at init.
- **`render.rs`** — Terminal output: `StreamingLine` (green during streaming, overwrite via `\r`, prints prefix once), `print_info`, `print_model`, `print_usage`, `print_session_entry`.
- **`keyring.rs`** — OS keychain wrapper. Service name: `"com.sakichan"`. Accounts: `"sensenova"` and `"deepseek"`. `resolve_keyring_value()` strips `ENC_KEYRING:` prefix, looks up keychain; if not found, prompts stdin and saves the input.

### `sakichan-gtk` (binary crate)

GTK4 desktop GUI frontend (~1960 lines). Metro-design dark green theme (`style.css`). Key design:

- **`main.rs`** — Single global tokio runtime via `OnceLock<Runtime>`. `AppState { session_store, memory_store, config, current_sid, pending_imgs, pending_files, is_generating }` in `Rc<RefCell<AppState>>`. `PanelCtx { chat_box, chat_scroll, sess_title, session_list, send_btn, dsv4f_btn }` — cloneable shared UI references.
- **Markdown rendering** — `build_markdown_widget()` uses `pulldown_cmark` to build a GTK4 widget tree: headings via Pango span, code blocks with copy button and language label, bold/italic/strikethrough/inline-code via Pango markup, ordered/unordered lists as prefixed labels.
- **Thinking blocks** — Collapsible `Revealer` with "▸ 深度思考" toggle. Shown for assistant messages with non-empty `reasoning_content`. Elapsed time shown in toggle label after generation.
- **Streaming** — `Msg` enum (Token/Think/Done/Fail) over `async_channel`. `spawn_stream_receiver()` throttles markdown re-renders to 180ms intervals; final render always runs on `Done`.
- **Branching UI** — `make_branch_nav()` inserts left/right navigation buttons before the first message at `active_depth` when multiple orders exist. Calls `set_active_branch` then reloads messages.
- **Context menu** — Right-click `Popover` on message bubbles: user messages get "修改" (edit mode), assistant messages get "重新生成" (regen). Edit mode replaces bubble with `TextView`; Enter submits via `do_send_branch()`.
- **Auto-title** — After first response if session is untitled, spawns background task with first 6 messages (truncated to 200 chars each), asks model for ≤20 char title, updates label and session list via `async_channel`.
- **Session list** — Right-click Popover per row: "删除会话", "压缩对话" (`start_compress`), "重新总结标题" (`start_retitle`). Results communicated from tokio thread to GTK main thread via `async_channel` + `glib::MainContext::spawn_local`.
- **Layout** — 1100×720 window. Overlay with main column + dim overlay + 272px sidebar revealer. Header bar with hamburger button and session title. Body: scrolled chat area, attachment revealer bar, input area. Sidebar: API key management section, session list with "+ 新建会话".
- **`style.css`** — Background `#0d0d0d`, accent `#1c5035`/`#1c4a31`. Classes: `.msg-bubble-user` (green-tinted, right-aligned), `.msg-bubble-assistant` (dark, left-border accent), `.thinking-box`, `.code-block`, `.branch-nav`, `.edit-input`, `.loading-spinner`, `.dim-overlay`.

### Data Flow

```
User CLI args → main() → handle_send_prompt()
  1. Resolve session (create/load/reuse)
  2. Apply branching (--deri/--order or auto: max_depth+1, order=1)
  3. Select model (SenseNova or DeepSeek based on --dsv4f + images)
  4. Build PipelineContext (load branch messages from SQLite → SessionContext)
  5. Call pipeline.run() or run_image_pipeline()
     a. Append user message to SessionContext
     b. Persist to SQLite messages table, update active branch
     c. Retrieve long-term memories → inject as system prompt
     d. Sliding-window truncation on message list
     e. Call ChatModel::chat_stream() → SSE HTTP POST
     f. Stream tokens via PipelineEvent callbacks → StreamingLine output
     g. Persist assistant response to SQLite
  6. Print token usage stats

GTK frontend follows the same pipeline.run() / run_image_pipeline() path,
with tokio::runtime::Runtime::spawn_blocking to call send_blocking from the GTK main thread.
```

## Key Design Decisions

- **Streaming output**: CLI uses `StreamingLine` (green, `\r` overwrite). GTK throttles markdown re-renders to 180ms; final render always on `Done`.
- **Branching**: Messages stored with `(depth, order)` coordinates. Default new message: `max_depth + 1`, order `1`. `--deri` creates parallel branches at a given depth; collision check (bails if order already has messages at that depth).
- **Conversation compression**: `--compressconv` reads the active branch messages, optionally prepends latest existing summary as reference context, calls the model, persists to `conversation_summaries`. Incremental: new summary references the previous one.
- **Keychain**: API keys in OS keychain, referenced in config as `ENC_KEYRING:com.sakichan/<name>`. `resolve_keyring_value()` prompts stdin on first use and saves the key.
- **Schema migration**: `initialize_tables()` uses `CREATE TABLE IF NOT EXISTS` followed by `PRAGMA table_info` → `ALTER TABLE ADD COLUMN` for backward-compatible additive migrations.
- **Two-stage image pipeline**: `--dsv4f TRUE` + images → SenseNova analyses image (standalone `desc_messages` list, dedicated system prompt), result injected as `"[图片描述]\n...\n\n[用户问题]\n..."` context prompt, then DeepSeek answers on full session context.

## Configuration

- Config file: `~/.config/sakichan/config.toml`
- API keys: OS keychain (keyring crate), stored as `com.sakichan/sensenova` and `com.sakichan/deepseek`
- Database: `~/.local/share/sakichan/sakichan.db` (SQLite, shared between CLI and GTK)
- Default config auto-generated with entries for sensenova-flash-lite, deepseek-v4-flash, and sensenova-u1-fast
