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

| Constant | Role | Value | Responsibility |
|---|---|---|---|
| `DSR1` | Engineer & Architect | `deepseek-r1:8b` | Analysis (Phase 1), planning (Phase 3), architect check (4c/4d), compile fix (4e), doc writing |
| `QWEN` | Programmer | `qwen2.5-coder:7b` | Context extraction (Phase 0), code generation (Phase 4a QWEN steps), code review (4b) |

Role constants: `ROLE_DSR1` and `ROLE_QWEN` are prepended to each model's prompts to enforce role identity.

Each plan step carries an `assigned_model` field (`"QWEN"` or `"DSR1"`); `resolve_model()` maps this string to the actual model constant. The user's `state.current_model` field is stored but does not affect which model runs — model selection is fully driven by the plan.

### Model option presets (`src/orchestrator.rs`)

Three named presets tune inference per use-case:

| Preset | temp | Used for |
|---|---|---|
| `dsr1_opts()` | 0.3 | All DSR1 calls (analysis, planning, architect, compile-fix) |
| `qwen_ctx_opts()` | 0.1 | Phase 0 context extraction, Phase 4b review |
| `qwen_gen_opts()` | 0.2 | Phase 4a code generation (QWEN steps) |

### Five-phase orchestration (`src/orchestrator.rs`)

```
Phase 0  gather_context()   – qwen extracts filenames from request, reads them, enriches prompt
Phase 1  Analysis           – DSR1 returns JSON: understanding, task_type (free-form), task_category
                               (new_project|modify_project|document|analysis|other), complexity 1-10,
                               gathered_info[], clarifications[]
Phase 2  Clarification      – shows gathered_info; up to 3 inline questions; extras auto-answered
Phase 3  Planning           – DSR1 produces steps[]: id, name, submodule_prompt, assigned_model,
                               files_to_create, verification
Phase 4  Execution          – per-step generate → review → fix loop, then architect check, then
                               cargo check fix loop (Rust only)
Phase 5  Wrap-up            – updates .sakichan.md and build.log; saves session JSON
```

#### Phase 4 sub-stages and retry limits

```
4a  Generate    – exec_model (from step.assigned_model) generates code
4b  Review      – QWEN reviews; up to REVIEW_MAX_ATTEMPTS=3 fix iterations per step
4c  Architect   – DSR1 checks cross-module interface correctness
4d  Rework      – DSR1 fixes issues flagged [MINOR]/[MAJOR]; up to ARCH_MAX_ITERATIONS=2
4e  Compile     – cargo check; up to CARGO_FIX_MAX=5 DSR1 fix iterations (Rust only)
```

Phase 4 suppresses `cargo check` failure output — errors go silently back into the prompt; only `✓ 编译通过` or `❌ 编译失败 after N attempts` is shown.

### Key invariants

- **Confirmation gate.** After Phase 3 (plan displayed), Sakichan always asks `[y/n/a]` before Phase 4 unless `edit_mode=true`. `y` = execute once; `n` = cancel; `a` = set `edit_mode=true` (equivalent to `/edit`, no further prompts). `/edit` command toggles `edit_mode` persistently for the session.
- **Compilation gate.** Each compile attempt only proceeds if `cargo check` passes; on failure, the error text (≤1500 chars) is appended to the prompt and retried.
- **Executor safety.** `src/executor.rs` blocks dangerous command strings (`rm -rf /`, `diskpart`, etc.) before spawning any shell.
- **Environment header.** Every AI prompt starts with a Windows 11 / PowerShell environment declaration, even though the host is Linux — this is intentional for the *target* environment the generated code runs in.
- **Git checkpoint.** One `git add -A && git commit` fires at the start of Phase 4 (before any file writes). `/undo [n]` runs `git reset --hard HEAD~n` to revert.
- **SYSTEM tags.** AI responses may emit `[SYSTEM:read_file path="..."]` or `[SYSTEM:list_files path="..."]`; `process_system_calls()` in the orchestrator intercepts and resolves these before parsing.

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
| `main.rs` | REPL loop, Ollama connection check, routes `/commands` vs orchestrator |
| `commands.rs` | 16 slash commands + bilingual i18n (zh_TW default, en alternative) |
| `display.rs` | Color constants, animated `Spinner` (shows elapsed time + tokens), `print_cmd_result` |
| `orchestrator.rs` | All five phases; model option presets; `gather_context`, JSON parsing, code-block extraction |
| `state.rs` | `AppState` (host, model, lang, edit_mode, usage, checkpoint_count); JSON persistence helpers |
| `ollama.rs` | Blocking HTTP to `/api/generate` (streaming), `/api/tags`; 300 s timeout |
| `executor.rs` | Cross-platform shell runner; returns `(ok, output, secs)` |
| `logger.rs` | Append-only markdown log in `.sakichan/build.log` |
| `rules.rs` | `.sakichan.md` load/create/update |

### JSON parsing

`parse_json_from_response` does bracket-depth counting to extract the first `{…}` block from raw model output (models often wrap JSON in prose or `<think>` tags). `extract_code_blocks` handles both ` ```lang filename="path" ` and bare ` ```lang ` fences with filename inference fallback.
