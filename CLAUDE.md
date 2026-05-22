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

Sakichan is a Rust CLI (v0.3.0) that wraps a local Ollama instance into a ten-phase AI coding assistant. The user types a free-form request; the orchestrator drives two models through requirement analysis, solution design, module planning, code generation, and compilation until `cargo check` passes.

### Two models, fixed roles

| Constant | Role | Value | Responsibility |
|---|---|---|---|
| `DSR1` | Engineer & Architect | `deepseek-r1:8b` | Solution design (C), direction confirmation (D), module planning (E), compile fix (I), overall evaluation (I), rules update (J) |
| `QWEN` | Programmer | `qwen2.5-coder:7b` | Request summarization (A), clarification (B), code generation (F), quick review (G) |

Role constants: `ROLE_DSR1` and `ROLE_QWEN` are prepended to each model's prompts to enforce role identity.

Each module carries an `assigned_model` field (`"QWEN"` or `"DSR1"`); `resolve_model()` maps this to the actual model constant. `state.current_model` is stored but does not affect model selection — it is fully driven by the module plan.

### Model option presets (`src/orchestrator.rs`)

| Preset | temp | Used for |
|---|---|---|
| `dsr1_opts()` | 0.3 | All DSR1 calls |
| `qwen_ctx_opts()` | 0.1 | Phase A summarization, Phase G review |
| `qwen_gen_opts()` | 0.2 | Phase F code generation (QWEN steps) |

### Ten-phase orchestration (`src/orchestrator.rs`)

```
Phase A  Summarize        – QWEN summarizes user request + project tree into a brief
Phase B  Clarification    – DSR1 asks up to 3 high-impact questions; extras auto-accepted
Phase C  Solution design  – DSR1 outputs [SOLUTION_DESIGN] (natural language, no code yet)
Phase D  Direction check  – DSR1 asks up to 2 fork questions; user can enter 'n' to restart C
Phase E  Module plan      – DSR1 outputs [MODULE_PLAN] with [MODULE: name] blocks
Phase F  Generation       – exec_model generates code per module (patch or full-file)
Phase G  Quick review     – QWEN checks against module spec; up to REVIEW_MAX_ATTEMPTS=3
Phase H  Merge check      – static check (no model): missing files, unresolved crate:: refs
Phase I  Overall eval     – cargo check (CARGO_FIX_MAX=5 DSR1 iterations); DSR1 architect
                             check; [MINOR] rework pass; [MAJOR] prompts restart to C
Phase J  Wrap-up          – updates .sakichan.md, writes build.log, saves session JSON,
                             git commits
```

Phases C–J run in a `'main_loop` that can restart up to `MAX_RESTART=3` times. Restart is triggered either by the user rejecting a [MAJOR] issue in Phase I, or by pressing `n` at the Phase D direction prompt.

#### Phase F/G retry and Phase I fix limits

```
F/G loop  Generate → Review → (fix and loop back)  up to REVIEW_MAX_ATTEMPTS=3 per module
I compile  cargo check → DSR1 fix                   up to CARGO_FIX_MAX=5 iterations
```

Phase I suppresses raw `cargo check` output — first 8 `error` lines are shown; only `✓ 编译通过` or `❌ 编译失败 after N attempts` is the final verdict.

### Module plan format

Phase E produces text with these markers (parsed by `parse_modules()`):

```
[MODULE_PLAN]
[MODULE: 模块名]
inputs: ...
outputs: ...
needs_compile: true/false
assigned_model: QWEN/DSR1
...
```

If no `[MODULE:]` markers are found, the full plan text is wrapped in a single fallback module.

### Key invariants

- **Confirmation gate.** After Phase E (module list displayed), Sakichan always asks `[y=执行 / n=取消 / a=始终执行]` before Phase F unless `edit_mode=true`. `a` sets `edit_mode=true` for the session (equivalent to `/edit`).
- **Source-file guards.** `write_or_patch_files()` rejects writes to source files if: (a) the first 10 lines look like markdown prose, or (b) new content is less than 30% of the existing line count.
- **Patch format.** Code blocks with lang `patch` use `---OLD--- / ---NEW--- / ---END---` markers; `apply_patch()` does verbatim substring replacement. Falls back to full write on mismatch.
- **Compilation gate.** `cargo check` runs in Phase I only when `any_compile=true`; on failure the error text (≤1500 chars) is appended to the fix prompt.
- **Executor safety.** `src/executor.rs` blocks dangerous command strings (`rm -rf /`, `diskpart`, etc.) before spawning any shell.
- **Environment header.** Every AI prompt starts with a Windows 11 / PowerShell environment declaration, even though the host is Linux — this is intentional for the *target* environment the generated code runs in.
- **Git checkpoints.** One `git add -A && git commit` fires at the start of Phase F (before any file writes). `/undo [n]` runs `git reset --hard HEAD~n` to revert. A second commit fires at Phase J.
- **SYSTEM tags.** AI responses may emit `[SYSTEM:read_file path="..."]`, `[SYSTEM:list_files path="..."]`, `[SYSTEM:grep pattern="..." path="..."]`, or `[SYSTEM:web_search query="..."]`; `process_system_calls()` in `call_model()` intercepts, resolves, and injects results into a follow-up prompt before returning.
- **Web search.** Uses DuckDuckGo Instant Answer API (`api.duckduckgo.com`), truncated to 1000 chars.

### Persistence layout (created at runtime)

```
.sakichan/
  history.txt          readline history
  usage.json           token counts per day + totals
  build.log            markdown audit log of every task
  sessions/{uuid}.json resumable session context
.sakichan.md           per-project rules (template auto-created; AI reads it each run)
```

### Slash commands (`src/commands.rs`)

16 commands: `/help`, `/models`, `/model <name>`, `/clear`, `/init`, `/load <file>`, `/usage`, `/sessions`, `/resume <id>`, `/export <id>`, `/edit`, `/lang <zh|en>`, `/undo [n]`, `/history`, `/diff`, `/exit`.

Languages: `zh_TW` (default) and `en`. The `t()` helper resolves i18n keys at runtime.

### Module responsibilities

| File | Role |
|---|---|
| `main.rs` | REPL loop, Ollama connection check, git status display, routes `/commands` vs orchestrator |
| `commands.rs` | 16 slash commands + bilingual i18n (zh_TW default, en alternative) |
| `display.rs` | Color constants, animated `Spinner` (shows elapsed time + tokens), `print_cmd_result`, `print_code_diff`, `print_change_summary` |
| `orchestrator.rs` | All ten phases; model option presets; `gather_context`, `parse_modules`, `write_or_patch_files`, `apply_patch`, JSON/code-block extraction, `call_model` |
| `state.rs` | `AppState` (host, model, lang, edit_mode, usage, checkpoint_count); JSON persistence helpers |
| `ollama.rs` | Blocking streaming HTTP to `/api/generate`; `/api/tags`; 300 s timeout |
| `executor.rs` | Cross-platform shell runner; returns `(ok, output, secs)` |
| `logger.rs` | Append-only markdown log in `.sakichan/build.log` |
| `rules.rs` | `.sakichan.md` load/create/update |

### Parsing utilities

`parse_json_from_response` does bracket-depth counting to extract the first `{…}` block from raw model output (models often wrap JSON in prose or `<think>` tags). `extract_code_blocks` handles both ` ```lang filename="path" ` and bare ` ```lang ` fences with filename inference fallback (detects `[package]`/`fn main`/first `.rs` mention).
