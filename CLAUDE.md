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

Sakichan is a Rust CLI (v0.4.1) that wraps a local LLM backend into a ten-phase AI coding assistant. The user types a free-form request; the orchestrator drives five role-slots through requirement analysis, solution design, module planning, code generation, and compilation until the verification command passes.

### Five-slot role system (`src/slots.rs`)

Model selection is fully driven by **slots**, not hardcoded model names. Each slot maps to a role with a fixed prompt identity and a configurable model assignment:

| Slot | Role | Default model | Phase responsibilities |
|---|---|---|---|
| `ProductOwner` | Requirements, acceptance | `qwen2.5-coder:7b` | A (summarize), B (clarify), J (wrap-up) |
| `Architect` | Solution design, module split | `deepseek-r1:8b` | C (design), D (direction), E (module plan), I (arch review) |
| `SeniorEngineer` | Complex logic, compile fix | `deepseek-r1:8b` | I (compile fix loop) |
| `Programmer` | Code generation | `qwen2.5-coder:7b` | F (generate, default slot) |
| `QA` | Code review | `qwen2.5-coder:7b` | G (quick review) |

`SlotRole::from_str()` accepts legacy model names (`qwen`, `dsr1`, `deepseek-r1:8b`, etc.) and maps them to slots for backward compatibility with old session files.

Slot resolution order (in `resolve_slot()`): `primary` → `fallback[]` → `models[]` → primary even if unavailable (avoids blocking when probe fails).

### Configuration (`src/config.rs`, `sakichan.conf`)

Config is TOML. Search order: `./sakichan.conf` → `$SAKICHAN_CONFIG` → `~/.config/sakichan/sakichan.conf`. A default is written to the user path on first run.

Key sections:

```toml
[general]
lang = "zh_TW"       # or "en"
edit_mode = false

[backend]
type = "ollama"      # or "llama_server"

[backend.ollama]
host = "http://localhost:11434"

[slots.Programmer]
primary = "qwen2.5-coder:7b"
fallback = ["deepseek-coder:6.7b"]
models = ["qwen2.5-coder:7b", "deepseek-coder:6.7b", "deepseek-r1:8b"]

[presets.programmer]
temperature = 0.2
top_p = 0.9

[slot_presets]
Programmer = "programmer"

[[verification]]     # custom override only — auto-detection runs first (see Phase I)

[[toolchain]]        # user-registered tools injected into prompts
name = "zig"
check_command = "zig version"
description = "Zig 编译器"
```

`SakichanConfig::get_preset_for_slot()` merges the named preset with any `preset_override` in the slot config.

### Toolchain detection (`src/main.rs`, `src/state.rs`)

At startup, `detect_toolchain()` probes `TOOLCHAIN_REGISTRY` (cargo, python3, node, tsc, go, zig, gcc, git, docker, …) plus user entries from `[[toolchain]]` in config. Results are stored in `AppState.toolchain_info: Vec<DetectedTool>` and injected into **every Phase prompt** via `state.toolchain_prompt_section()` — a formatted `## 当前环境可用工具` section.

AI models in every phase see which tools are available in the environment and can make decisions accordingly.

### Backend trait (`src/backend/mod.rs`)

```rust
pub trait LlmBackend: Send + Sync {
    fn generate_complete(&self, model: &str, prompt: &str, options: &ModelOptions) -> Result<(String, UsageStats)>;
    fn list_models(&self) -> Vec<String>;
    fn backend_name(&self) -> &'static str;
}
```

`create_backend()` is the factory. Implemented backends:
- `OllamaBackend` (`src/backend/ollama.rs`) — blocking streaming HTTP, 300 s timeout
- `LlamaServerBackend` (`src/backend/llama_server.rs`) — spawns `llama-server` subprocess; has `cached_model: Mutex<Option<String>>` to skip model restarts when the same model is requested again; kills + restarts subprocess only on actual model switch

### Phase → slot mapping (`src/orchestrator.rs`)

```
Phase A  Summarize        – ProductOwner: user request + project tree → brief
Phase B  Clarification    – ProductOwner: up to 3 questions; extras auto-accepted
Phase C  Solution design  – Architect: outputs [SOLUTION_DESIGN]
Phase D  Direction check  – Architect: up to 2 fork questions; 'n' restarts C
Phase E  Module plan      – Architect: outputs <SAKICHAN:MODULE> blocks
Phase F  Generation       – module's assigned_slot (default: Programmer)
Phase G  Quick review     – QA: checks against module spec; up to REVIEW_MAX_ATTEMPTS=3
Phase H  Merge Check      – static (no model): missing files, unresolved crate:: refs
Phase I  Overall eval     – SeniorEngineer: compile fix (CARGO_FIX_MAX=5);
                             Architect: arch review; [MINOR] rework; [MAJOR] → restart to C
Phase J  Wrap-up          – ProductOwner: updates .sakichan.md, writes build.log,
                             saves session JSON, git commits
```

Phases C–J run in a `'main_loop` that restarts up to `MAX_RESTART=3` times.

### Module plan format — `<SAKICHAN:...>` markup

Phase E outputs modules using a unified XML-style markup (parsed by `parse_sakichan_tags()` in `handoff.rs`). The old `[MODULE:]` bracket format is still parsed as a fallback.

```xml
<SAKICHAN:MODULE name="模块名" slot="Programmer" compile="true">
  <SAKICHAN:INPUT>输入规范</SAKICHAN:INPUT>
  <SAKICHAN:OUTPUT>输出规范</SAKICHAN:OUTPUT>
  <SAKICHAN:CONSTRAINT type="HARD">密码必须用 argon2 哈希</SAKICHAN:CONSTRAINT>
  <SAKICHAN:CONSTRAINT type="SOFT">偏好迭代器而非循环</SAKICHAN:CONSTRAINT>
  <SAKICHAN:VERIFY>cargo check passes</SAKICHAN:VERIFY>
</SAKICHAN:MODULE>
```

`parse_modules()` tries SAKICHAN format first, falls back to `[MODULE:]` for backward compat. No `<SAKICHAN:MODULE>` found → entire plan text becomes a single fallback module.

`sakichan_tag_to_module()` converts a `SakichanTag::Module` to the existing `Module` struct used throughout the orchestrator.

### Handoff types (`src/handoff.rs`)

Structured types that carry data between phases:
- `Module` — Phase E → F/G (spec, slot, constraints, verification criteria)
- `ReviewResult` + `Issue` — Phase G → F (issues with severity Major/Minor/Info)
- `MajorRestartReport` — Phase I → C (why restart, what to avoid, constraints for redesign)
- `SakichanTag` / `parse_sakichan_tags()` — unified markup parser for all inter-phase text
- `build_generation_prompt()` / `build_fix_prompt()` — construct full prompts from these structs

### Phase I: extension-based verification

Phase I auto-detects which verification command to run based on the files generated and the toolchain:

| Files | Tool available | Command |
|---|---|---|
| `*.rs` | `cargo` | `cargo check 2>&1` |
| `*.py` | `python3` / `python` | `python3 -m py_compile <files>` |
| `*.ts` / `*.tsx` | `tsc` | `tsc --noEmit 2>&1` |
| `*.go` | `go` | `go build ./... 2>&1` |
| `*.zig` | `zig` | `zig ast-check <files>` |
| no match | — | skip (inform user) |

Config `[[verification]]` entries are **custom overrides** — they take priority over auto-detection but are not required.

`run_verification()` does a single check and returns `(ok, error_text)`. The retry loop (up to `CARGO_FIX_MAX=5`) is in the orchestrator, interleaved with SeniorEngineer AI fixes.

### Key invariants

- **Confirmation gate.** After Phase E, Sakichan always asks `[y=执行 / n=取消 / a=始终执行]` before Phase F unless `edit_mode=true`. `a` sets `edit_mode=true` for the session.
- **Source-file guards.** `write_or_patch_files()` applies two guards to ALL files (no extension whitelist): (1) first 10 lines look like markdown prose → skip; (2) new content < 30% of existing line count → skip. The old `allow_source` / extension-whitelist gate is removed.
- **Patch format.** Code blocks with lang `patch` use `---OLD--- / ---NEW--- / ---END---` markers; `apply_patch()` does verbatim substring replacement. Falls back to full write on mismatch.
- **Spinner uniqueness.** One spinner line is active at a time, always overwritten in place via `\r`. Model name shown as a hint in the spinner via `Spinner::set_hint(model)` — no separate model-switch println. `LlamaServerBackend::load_model()` is silent.
- **Executor safety.** `src/executor.rs` blocks dangerous command strings (`rm -rf /`, `diskpart`, etc.) before spawning any shell.
- **Environment header.** Every AI prompt starts with a Windows 11 / PowerShell environment declaration even though the host is Linux — intentional for the *target* environment the generated code runs in.
- **Git checkpoints.** `git add -A && git commit` fires at the start of Phase F (before file writes) and again at Phase J. `/undo [n]` runs `git reset --hard HEAD~n`.
- **SYSTEM tags.** AI responses may emit `[SYSTEM:read_file path="..."]`, `[SYSTEM:list_files path="..."]`, `[SYSTEM:grep pattern="..." path="..."]`, or `[SYSTEM:web_search query="..."]`; `process_system_calls()` in `call_model()` intercepts, resolves, and injects results into a follow-up prompt.
- **Web search.** DuckDuckGo Instant Answer API (`api.duckduckgo.com`), truncated to 1000 chars.

### AppState (`src/state.rs`)

`AppState` holds `config: SakichanConfig`, `slot_assignments: HashMap<String, String>` (resolved at startup), `toolchain_info: Vec<DetectedTool>` (detected at startup), `edit_mode`, `lang`, `usage`, and `checkpoint_count`.

`toolchain_prompt_section()` formats `toolchain_info` into a `## 当前环境可用工具` string suitable for prepending to any prompt.

### Persistence layout

```
sakichan.conf              per-project config (overrides user config)
~/.config/sakichan/
  sakichan.conf            user-level config (auto-created on first run)
.sakichan/
  history.txt              readline history
  usage.json               token counts per day + totals
  build.log                markdown audit log of every task
  sessions/{uuid}.json     resumable session context
.sakichan.md               per-project rules (template auto-created; AI reads it each run)
```

### Slash commands (`src/commands.rs`)

16 commands: `/help`, `/models`, `/model <name>`, `/clear`, `/init`, `/load <file>`, `/usage`, `/sessions`, `/resume <id>`, `/export <id>`, `/edit`, `/lang <zh|en>`, `/undo [n]`, `/history`, `/diff`, `/exit`.

`/model <name>` accepts slot names (`architect`, `programmer`, etc.) or legacy model names — routed through `SlotRole::from_str()`.

### Module responsibilities

| File | Role |
|---|---|
| `main.rs` | REPL loop, backend + connection check, slot assignment, `detect_toolchain()` at startup; routes `/commands` vs orchestrator |
| `commands.rs` | 16 slash commands + bilingual i18n (zh_TW default, en alternative); `t()` resolves keys at runtime |
| `config.rs` | `SakichanConfig` + all sub-structs including `ToolchainEntry`; TOML load/save; verification strategy detector logic |
| `slots.rs` | `SlotRole` enum; `resolve_slot()`; `build_slot_assignments()`; `probe_ollama_models()` |
| `handoff.rs` | `Module`, `ReviewResult`, `MajorRestartReport`; `SakichanTag`, `parse_sakichan_tags()`, `sakichan_tag_to_module()`; `build_generation_prompt()`, `build_fix_prompt()` |
| `backend/mod.rs` | `LlmBackend` trait, `ModelOptions`, `UsageStats`, `create_backend()` factory |
| `backend/ollama.rs` | Blocking streaming HTTP to Ollama `/api/generate`; 300 s timeout |
| `backend/llama_server.rs` | Spawns `llama-server` subprocess; `cached_model` cache skips restart when same model; HTTP to `/completion` |
| `orchestrator.rs` | All ten phases; `parse_modules()`, `write_or_patch_files()`, `apply_patch()`, `call_model()`, `process_system_calls()`, `run_verification()`, `detect_verification_command()` |
| `state.rs` | `AppState`; `DetectedTool`; `toolchain_prompt_section()`; JSON persistence helpers for usage and sessions |
| `display.rs` | Color constants, `Spinner` with `set_hint()` for single-line model display; `print_cmd_result`, `print_code_diff` |
| `executor.rs` | Cross-platform shell runner; returns `(ok, output, secs)` |
| `logger.rs` | Append-only markdown log in `.sakichan/build.log` |
| `rules.rs` | `.sakichan.md` load/create/update |

### Parsing utilities

`parse_json_from_response`: bracket-depth counting to extract the first `{…}` block from raw model output (models often wrap JSON in prose or `<think>` tags).

`extract_code_blocks`: handles both ` ```lang filename="path" ` and bare ` ```lang ` fences with filename inference fallback (detects `[package]`/`fn main`/first `.rs` mention).
