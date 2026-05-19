# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo check          # fast type/borrow check (used internally by orchestrator loops)
cargo build          # debug build
cargo build --release
cargo run            # runs against current directory as work_dir
```

No test suite exists yet. Lint: `cargo check` doubles as the lint step; no clippy CI is configured.

## Architecture

Sakichan is a Rust CLI that wraps a local Ollama instance into a five-phase AI coding assistant. The user types a free-form request; the orchestrator drives two models through analysis, planning, code generation, and compilation until `cargo check` passes.

### Two models, fixed roles

| Constant | Value | Used for |
|---|---|---|
| `DSR1` | `deepseek-r1:8b` | Phase 1 analysis, Phase 3 planning, DSR1 fallback in Phase 4 |
| `QWEN` | `qwen2.5-coder:7b` | Phase 0 context extraction, Phase 4 initial code generation |

The user's `state.current_model` only influences which model Phase 4 *starts* with (via `step_model = current_model`). If `analysis.complexity >= 7`, `current_model` is promoted to DSR1 before Phase 4.

### Five-phase orchestration (`src/orchestrator.rs`)

```
Phase 0  gather_context()   – qwen extracts filenames from request, reads them, enriches prompt
Phase 1  Analysis           – DSR1 returns JSON: understanding, complexity 1-10, gathered_info[], clarifications[]
Phase 2  Clarification      – shows gathered_info; up to 2 inline questions; 3rd+ auto-answered
Phase 3  Planning           – DSR1 produces steps[]: id, name, description, files_to_create, verification
Phase 4  Execution          – qwen (≤5 retries) → DSR1 (≤10 retries); cargo check after each write
Phase 5  Final build        – cargo build --release; updates .sakichan.md and build.log
```

Phase 4 suppresses `cargo check` failure output — errors go silently back into the prompt; only `✅ 编译通过` or `❌ 编译失败 after N attempts` is shown.

### Key invariants

- **Read-only by default.** `state.edit_mode = false` at startup; Phase 4 file writes are skipped unless the user runs `/edit`.
- **Compilation gate.** Each step only proceeds if `cargo check` passes; on failure, the error text (≤1500 chars) is appended to the prompt and retried.
- **Executor safety.** `src/executor.rs` blocks dangerous command strings (`rm -rf /`, `diskpart`, etc.) before spawning any shell.
- **Environment header.** Every AI prompt starts with a Windows 11 / PowerShell environment declaration, even though the host is Linux — this is intentional for the *target* environment the generated code runs in.

### Persistence layout (created at runtime)

```
.sakichan/
  history.txt          readline history
  usage.json           token counts per day + totals
  build.log            markdown audit log of every task
  sessions/{uuid}.json resumable session context
.sakichan.md           per-project rules (template auto-created; AI reads it each run)
```

### Module responsibilities

| File | Role |
|---|---|
| `main.rs` | REPL loop, Ollama connection check, routes `/commands` vs orchestrator |
| `commands.rs` | 13 slash commands + bilingual i18n (zh_TW default, en alternative) |
| `display.rs` | Color constants, animated `Spinner` (shows elapsed time + tokens), `print_cmd_result` |
| `orchestrator.rs` | All five phases; `gather_context`, JSON parsing, code-block extraction |
| `state.rs` | `AppState` (host, model, lang, edit_mode, usage); JSON persistence helpers |
| `ollama.rs` | Blocking HTTP to `/api/generate` (streaming), `/api/tags`; 300 s timeout |
| `executor.rs` | Cross-platform shell runner; returns `(ok, output, secs)` |
| `logger.rs` | Append-only markdown log in `.sakichan/build.log` |
| `rules.rs` | `.sakichan.md` load/create/update |

### JSON parsing

`parse_json_from_response` does bracket-depth counting to extract the first `{…}` block from raw model output (models often wrap JSON in prose or `<think>` tags). `extract_code_blocks` handles both ` ```lang filename="path" ` and bare ` ```lang ` fences with filename inference fallback.
