# Sakichan

Sakichan 是一个高性能 AI 客户端，支持多模型接入、长短期记忆、多模态图像理解和对话分支管理。

## Features

- **多模型支持** — SenseNova Flash-Lite（默认多模态）、DeepSeek V4 Flash（思考模式）
- **多模态图像理解** — 直接 SenseNova 图片问答，或 `--dsv4f TRUE` 启用两阶段分析
- **对话分支** — 通过 `--deri` 和 `--order` 在任意对话层级创建平行分支
- **长期记忆** — CJK 感知的关键词检索，自动注入相关记忆到对话上下文
- **文本文件附件** — `--file` 读取文本文件作为上下文
- **256K 上下文窗口** — 滑动窗口截断，保留关键历史
- **Keychain 安全存储** — API Key 存储在 OS 密钥链中，不在配置文件明文保存

## Quick Start

### 1. 配置

首次运行会自动生成默认配置文件，或复制示例配置：

```bash
cp config/sakichan.example.toml ~/.config/sakichan/config.toml
```

### 2. 设置 API Key

```bash
# 设置 API Key（存储在系统 keychain）
sakichan --token sk-your-api-key-here

# 清除 API Key
sakichan --token clear
```

### 3. 开始对话

```bash
# 创建新会话
sakichan --session create

# 文字对话
sakichan --session <ID> --prompt "你好"

# 图片对话
sakichan --session <ID> --prompt "这是什么" --image photo.jpg

# 文件上下文
sakichan --session <ID> --prompt "总结这个文件" --file notes.txt

# 启用 DeepSeek 思考模式
sakichan --session <ID> --prompt "..." --dsv4f TRUE --thinking TRUE
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
| `--token <key>` | 设置 API Key，或 `clear` 清除 |

## Model Selection

默认使用 SenseNova Flash-Lite（多模态，文字和图片均直接处理）。

| 模式 | 文字 | 图片 |
|------|------|------|
| 默认 | SenseNova Flash-Lite | SenseNova Flash-Lite 直接回答 |
| `--dsv4f TRUE` | DeepSeek V4 Flash | SenseNova 分析 → DeepSeek 回答 |

## Configuration

配置文件路径：`~/.config/sakichan/config.toml`

```toml
[models]
default_chat = "sensenova-flash-lite"
default_image = "sensenova-u1-fast"

[models.sensenova-flash-lite]
api_base = "https://token.sensenova.cn/v1/chat/completions"
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

API Key 使用 `ENC_KEYRING:com.sakichan/<name>` 格式引用系统 keychain，也可通过 `--token` 命令管理。

## Architecture

```
sakichan-core (library)      sakichan-cli (binary)
┌─────────────────────┐      ┌────────────────────┐
│ config.rs           │      │ main.rs            │
│ models/             │◄────►│   CLI args         │
│   traits.rs         │      │   model selection  │
│   sensenova_flash   │      │   image pipeline   │
│   deepseek_v4_flash │      │   branching logic  │
│   sensenova_u1_fast │      │ render.rs          │
│ session/            │      │   StreamingLine    │
│   store.rs (SQLite) │      │ keyring.rs         │
│   context.rs        │      │   OS keychain      │
│ memory/             │      └────────────────────┘
│   store.rs          │
│   retrieval.rs      │
│ pipeline.rs         │
│ error.rs            │
└─────────────────────┘
```

## Data Flow

```
CLI args → resolve session → apply branching → select model
  → build PipelineContext (load messages from SQLite)
  → pipeline.run() or two-stage image pipeline
     → append user message → persist to SQLite
     → retrieve memories → inject as context
     → sliding-window truncation
     → ChatModel::chat_stream() → SSE HTTP POST
     → stream tokens → StreamingLine output
     → persist assistant response
  → print usage stats
```

## Branching

对话支持树状分支结构。每条消息有 `(depth, order)` 坐标：

```
depth 0: [user] "你好" → [assistant] "你好！"
         │
depth 1: ├─ order 1: [user] "今天天气如何？" → ...
         ├─ order 2: [user] "你会做什么？"    → ... (--deri 1 --order 2)
         └─ order 3: [user] "讲个笑话"       → ... (--deri 1 --order 3)
```

## Build

```bash
# 编译
cargo build

# 运行测试
cargo test

# 发布编译
cargo build --release
```

## Dependencies

- **Rust** edition 2021
- **tokio** — async runtime
- **clap** — CLI argument parsing
- **reqwest** — HTTP client (SSE streaming)
- **rusqlite** — SQLite database
- **keyring** — OS keychain
- **serde** / **serde_json** — serialization
- **colored** — terminal output colors
- **base64** — image encoding
