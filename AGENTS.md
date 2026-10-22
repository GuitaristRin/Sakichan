# AGENTS.md

<!-- BEGIN workbench-pointer · v1 · 勿手改本块（由 GuitaristRin/Projects 统一维护；改模板请改 templates/repo-AGENTS-block.md） -->
> **本文件只写本仓特有的规则。跨仓通用约定不在本仓重复**，唯一真源是工作台文档库：
> - Windows：`%USERPROFILE%\Documents\Projects\AGENTS.md`
> - Linux：`~/projects/AGENTS.md` → `~/projects/Arcturius/AGENTS.md`（按序取第一个存在的）
> - 远端：<https://github.com/GuitaristRin/Projects/blob/main/AGENTS.md>
>
> 动手之前先读它。**读不到就在报告里写明「工作台未读到」**，然后按本文件执行，不要猜。
> 新发现的通用规则先回流工作台（`docs/09-回流与同步.md`），本文件只留引用，不抄正文。
> 冲突优先级：用户当前明确指令 > 本仓特有规则 > 工作台通用规则。
<!-- END workbench-pointer -->

## Build & verify

```bash
./gradlew :app:compileDebugKotlin   # fast typecheck (no APK)
./gradlew :app:assembleDebug        # debug APK -> app/build/outputs/apk/debug/
./gradlew :app:assembleRelease      # release APK (needs signing config)
```

No test suite exists. Verification = `compileDebugKotlin` (zero errors expected; warnings are pre-existing and acceptable).

## Toolchain constraints (do not upgrade blindly)

- **Kotlin 1.9.24** is locked. `kotlinx-serialization` must stay **1.6.3** (1.7+ requires Kotlin 2.0+).
- Compose compiler **1.5.14**, Compose BOM **2024.12.01**, AGP **8.5.0** — these are interlocked with Kotlin 1.9.24.
- JDK on this machine is 26; AGP 8.5.0 emits Kotlin daemon warnings under it — safe to ignore.
- `compileSdk = 36`, `minSdk = 26`. `jvmTarget = 11`.

## Kanesumi composite build

UI library is a **sibling repo** at `../Kanesumi`, wired via `includeBuild` in `settings.gradle.kts` with dependency substitution to `io.github.takahashirinta:kanesumi-*`. The `../Kanesumi` path is hardcoded — cloning elsewhere breaks the build. Changing public Kanesumi APIs also affects its other consumer, **Ncrust**.

Metro UI conventions (enforced by Kanesumi, not negotiable):
- **Zero Material3**: no `material3`/`material` deps. Only `material-icons-core:1.7.5` for `ImageVector` assets.
- No rounded corners, no shadows, no borders unless information requires them.
- Read insets via `rememberMetroInsets()`, not raw `WindowInsets.*`.
- Animations use `:kanesumi-anim` Sokuou presets, not ad-hoc `tween()`.

## Architecture

Native Android app (Kotlin + Jetpack Compose). Package: `com.sakichan.se`. Single `:app` module, no submodules.

Two-layer secretary protocol (core logic, not just UI):
- **Thinking layer** (`core/agent/ThinkingAgent.kt`): multi-turn LLM loop (max 30 iter) with marker tools (`cmd`/`waitnext`/`reply`/`start_task`/`end`) + read-only skills (`list_directory`/`read_file`/`search_files`). Phased budget: first 1/3 = explore (full tools), last 2/3 = converge (skills removed). Secretary model is DeepSeek V4 Flash via SenseNova, `reasoning_effort:"none"` — depth comes from the loop, not native reasoning.
- **Surface layer** (`ui/chat/ChatViewModel.kt`): orchestrates thinking -> proposal (user approves) -> opencode execution -> adversarial review -> summary.

`reply(feedback)` is the WeChat-style instant acknowledgment mechanism — the first thinking turn **must** produce visible feedback. `ThinkingAgent` has code-level fallbacks to guarantee this even when the model skips it.

Models are configured via `ModelRoster` (THINKING/SUMMARY/REVIEW roles) in `AppConfigRepository.kt` — currently all DSV4F, but the structure allows per-role model swap without code changes.

## opencode server integration

App talks to a PC-side `opencode serve --hostname 0.0.0.0 --port 4096 --mdns` over LAN HTTP+SSE. Key gotchas (verified against opencode 1.18.x):
- **Per-session SSE endpoint is empty** — real events come from `GET /event?directory=<workdir>` (directory-scoped, not session-scoped).
- **Completion detection**: poll `GET /session/:id/message`, look for `step-finish` part. 180s timeout in ViewModel, 300s in `TaskMonitorService`.
- **Create session with `directory` query param** — omitting it can land on a stale/deleted project dir → ENOENT.
- `opencodeSessionId` is validated via `getSession()` before reuse (PC restart invalidates old sessions).

## Robustness: foreground service

`TaskMonitorService` (foreground service) keeps the process alive during opencode task execution so the "walk away to get coffee" scenario works. If the app is killed, the service persists the result to `PendingCompletionStore`; on next `restoreLastSession()` the ViewModel runs `checkPendingCompletion()` to recover — either from the store or by re-polling opencode directly.

## Persistence

- **DataStore Preferences** (`ChatHistoryRepository`): chat history + session metadata, keyed `chat_<machineId>_<sessionId>`. No Room (data is small, kotlinx-serialization is sufficient).
- **EncryptedSharedPreferences** (`AppConfigRepository`): SenseNova API key only.
- **SharedPreferences** (`PendingCompletionStore`): single write-once-read-once recovery record.
- `SessionContext.buildMessagesForModel()` implements sliding-window truncation but is **not called anywhere** — `messages()` returns raw history. Long conversations can hit API token limits.
- `ChatItem.Task` and `ChatItem.Proposal` are transient — not persisted. Only `User` and `Secretary` survive session restore.

## Conventions

- **No comments** in code unless explicitly requested by the user.
- Chinese is used in user-facing strings, prompts, and some log messages.
- Debug build has `.debug` applicationId suffix (`com.sakichan.se.debug`).
- `local.properties` (gitignored) points `sdk.dir` to the Android SDK.
- Debug `ChatApiClient`/SSE logs are verbose (`Log.d("SSE", ...)`) — normal during development.
