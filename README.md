# Sakichan

Sakichan 是一个高性能 AI 客户端，支持多模型接入、长短期记忆、多模态图像理解和对话分支管理。提供 CLI 和 GTK4 桌面 GUI 两种前端。

## Features

- **多模型支持** — SenseNova Flash-Lite（默认多模态）、DeepSeek V4 Flash（思考模式）
- **多模态图像理解** — 直接 SenseNova 图片问答，或 `--dsv4f TRUE` 启用两阶段分析
- **对话分支** — 通过 `--deri` 和 `--order` 在任意对话层级创建平行分支，GUI 提供左右导航按钮
- **对话压缩** — `--compressconv` 对当前分支全部消息生成总结，可增量叠加，持久化存储
- **长期记忆** — CJK 感知的关键词检索，自动注入相关记忆到对话上下文
- **文本文件附件** — `--file` 读取文本文件作为上下文
- **256K 上下文窗口** — 滑动窗口截断，保留关键历史
- **Keychain 安全存储** — API Key 存储在 OS 密钥链中，不在配置文件明文保存
- **GTK4 桌面 GUI** — 深绿色 Metro 主题，Markdown 渲染，思考块折叠展开，流式输出，自动生成会话标题

## Quick Start

### 1. 配置

首次运行会自动生成默认配置文件，或复制示例配置：

```bash
cp config/sakichan.example.toml ~/.config/sakichan/config.toml
```

### 2. 设置 API Key

```bash
# CLI：设置 API Key（存储在系统 keychain）
sakichan-cli --token sk-your-api-key-here

# 清除 API Key
sakichan-cli --token clear
```

GUI 中可在左侧边栏 API KEY 区域直接输入和保存。

### 3. 开始对话

**CLI：**

```bash
# 创建新会话
sakichan-cli --session create

# 文字对话
sakichan-cli --session <ID> --prompt "你好"

# 图片对话
sakichan-cli --session <ID> --prompt "这是什么" --image photo.jpg

# 文件上下文
sakichan-cli --session <ID> --prompt "总结这个文件" --file notes.txt

# 启用 DeepSeek 思考模式
sakichan-cli --session <ID> --prompt "..." --dsv4f TRUE --thinking TRUE

# 总结当前分支对话并保存
sakichan-cli --session <ID> --compressconv
```

**GUI：**

```bash
sakichan-gtk
```

## CLI Reference

| Flag | Description |
|------|-------------|
| `--session <ID>` | 会话 ID, `create`, `list`, `suspend` |
| `--prompt <text>` | 发送消息 |
| `--image <path>` | 图片路径（可多次使用） |
| `--file <path>` | 文本文件路径（可多次使用） |
| `--dsv4f TRUE/FALSE` | 启用 DeepSeek V4 Flash（默认 FALSE） |
| `--thinking TRUE/FALSE` | 启用 DeepSeek 思考模式 |
| `--deri <depth>` | 在指定深度创建分支 |
| `--order <n>` | 分支 order 编号 |
| `--delete` | 删除会话 |
| `--summary` | 生成/更新会话标题 |
| `--compressconv` | 总结并存储当前分支全部对话 |
| `--token <key>` | 设置 API Key，或 `clear` 清除 |

## Model Selection

默认使用 SenseNova Flash-Lite（多模态，文字和图片均直接处理）。

| 模式 | 文字 | 图片 |
|------|------|------|
| 默认 | SenseNova Flash-Lite | SenseNova Flash-Lite 直接回答 |
| `--dsv4f TRUE` | DeepSeek V4 Flash | SenseNova 分析 → DeepSeek 回答 |

## GTK4 GUI

`sakichan-gtk` 是原生 GTK4 桌面客户端，与 CLI 共享同一 SQLite 数据库和配置文件。

主要功能：

- **Markdown 渲染** — 标题、代码块（含复制按钮）、粗体/斜体、列表、内联代码
- **思考块** — DeepSeek 深度思考内容折叠显示，点击展开，显示思考用时
- **流式输出** — Token 实时显示，Markdown 每 180ms 重渲一次，完成后最终渲染
- **分支导航** — 同一深度有多个分支时，在分支起点插入左右方向键切换
- **右键菜单** — 用户消息：修改（进入行内编辑模式）；助手消息：重新生成
- **自动标题** — 首次收到回复后自动生成会话标题（≤20字）
- **侧边栏** — API Key 管理、会话列表（支持删除、压缩对话、重新生成标题）
- **图片/文件附件** — 工具栏按钮附加图片和文本文件

## Configuration

配置文件路径：`~/.config/sakichan/config.toml`

```toml
[models]
default_chat = "sensenova-flash-lite"
default_image = "sensenova-u1-fast"

[models.sensenova-flash-lite]
api_base = "https://token.sensenova.cn/v1/chat/completions"
api_key = "ENC_KEYRING:com.sakichan/sensenova"

[models.sensenova-u1-fast]
api_base = "https://api.sensenova.cn/v1/images/generations"
api_key = "ENC_KEYRING:com.sakichan/sensenova"

[models.deepseek-v4-flash]
api_base = "https://token.sensenova.cn/v1/chat/completions"
api_key = "ENC_KEYRING:com.sakichan/deepseek"
extra_params = { reasoning_effort = "medium" }

[database]
path = "~/.local/share/sakichan/sakichan.db"

[memory]
auto_summarize = true
retrieval_top_k = 5
```

API Key 使用 `ENC_KEYRING:com.sakichan/<name>` 格式引用系统 keychain，也可通过 `--token` 命令或 GUI 侧边栏管理。

## Architecture

```
sakichan-core (library)
┌──────────────────────────────────┐
│ config.rs                        │
│ models/                          │
│   traits.rs (ChatModel, types)   │
│   sensenova_flash_lite.rs        │
│   deepseek_v4_flash.rs           │
│   sensenova_u1_fast.rs           │
│ session/                         │
│   store.rs (SQLite)              │
│   context.rs (sliding window)    │
│ memory/                          │
│   store.rs                       │
│   retrieval.rs (CJK keywords)    │
│ pipeline.rs                      │
│ error.rs                         │
└──────────┬───────────────────────┘
           │
    ┌──────┴──────┐
    ▼             ▼
sakichan-cli   sakichan-gtk
(CLI binary)   (GTK4 GUI binary)
```

## Data Flow

```
CLI args → resolve session → apply branching → select model
  → build PipelineContext (load branch messages from SQLite)
  → pipeline.run() or run_image_pipeline()
     → append user message → persist to SQLite
     → retrieve memories → inject as context
     → sliding-window truncation
     → ChatModel::chat_stream() → SSE HTTP POST
     → stream tokens → output
     → persist assistant response
  → print usage stats
```

## Conversation Compression

`--compressconv` 对当前分支（以 `active_depth` / `active_order` 为准）的全部消息调用模型生成总结，并将结果持久化到 SQLite 的 `conversation_summaries` 表中。

```bash
sakichan-cli --session <ID> --compressconv
```

- **增量叠加** — 若已有历史总结，生成时自动将上一条总结作为参考上下文，确保新总结准确延续
- **存储字段** — `content`（总结内容）、`depth`、`order_at`（分支坐标）、`messages_count`（消息数量）、`summarized_at`（生成时间 UTC Unix 时间戳）
- **级联删除** — 会话删除时所有对应总结自动清除

GUI 侧边栏右键会话行可触发"压缩对话"。

## Branching

对话支持树状分支结构。每条消息有 `(depth, order)` 坐标：

```
depth 0: [user] "你好" → [assistant] "你好！"
         │
depth 1: ├─ order 1: [user] "今天天气如何？" → ...
         ├─ order 2: [user] "你会做什么？"    → ... (--deri 1 --order 2)
         └─ order 3: [user] "讲个笑话"       → ... (--deri 1 --order 3)
```

GUI 在有多个分支时于分支起点显示 ← → 导航按钮。

## Build

```bash
# 编译所有 crates
cargo build

# 仅编译 CLI
cargo build -p sakichan-cli

# 仅编译 GTK GUI（需要系统安装 gtk4 开发库）
cargo build -p sakichan-gtk

# 运行测试
cargo test

# 发布编译
cargo build --release
```

GTK4 开发库安装（Arch Linux）：

```bash
sudo pacman -S gtk4
```

## Dependencies

- **Rust** edition 2021
- **tokio** — async runtime
- **clap** — CLI argument parsing
- **reqwest** — HTTP client (SSE streaming)
- **rusqlite** — SQLite database (bundled, statically linked)
- **keyring** — OS keychain
- **serde** / **serde_json** — serialization
- **colored** — terminal output colors
- **base64** — image encoding
- **gtk4** / **gdk4** / **glib** / **gio** — GTK4 GUI (sakichan-gtk only)
- **pulldown-cmark** — Markdown parsing (sakichan-gtk only)
- **async-channel** — tokio↔GTK thread communication (sakichan-gtk only)
