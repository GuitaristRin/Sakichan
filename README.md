# 🌸 Saki-chan v0.4.0

> 角色驱动的本地 AI 编程助手 / Role-driven local AI coding assistant

Saki-chan 将本地 Ollama 模型编排为一支虚拟开发团队，通过十阶段工作流把自由文本需求转化为通过编译的代码。

---

## 快速开始 / Quick Start

```bash
# 需要 Ollama 在本地运行
ollama pull deepseek-r1:8b
ollama pull qwen2.5-coder:7b

cargo build --release
./target/release/sakichan
```

首次运行自动在 `~/.config/sakichan/sakichan.conf` 生成配置文件。

---

## 核心架构 / Architecture

### 五大插槽（角色驱动）

| 插槽 | 默认模型 | 负责阶段 |
|------|---------|---------|
| `ProductOwner` | qwen2.5-coder:7b | A（需求整理）、B（澄清）、J（收束） |
| `Architect` | deepseek-r1:8b | C（方案设计）、D（方向确认）、E（模块规划）、I（架构审查） |
| `SeniorEngineer` | deepseek-r1:8b | F（复杂模块）、I（编译诊断修复） |
| `Programmer` | qwen2.5-coder:7b | F（代码生成，默认） |
| `QA` | qwen2.5-coder:7b | G（快速评估）、H（合并检查） |

### 十阶段工作流

```
A  整理需求      ProductOwner  →  需求摘要
B  初步澄清      ProductOwner  →  向用户提问（最多3题）
C  方案设计      Architect     →  [SOLUTION_DESIGN] 自然语言方案
D  方向确认      Architect     →  关键岔路确认（最多2题）
E  模块规划      Architect     →  [MODULE_PLAN] 含约束/验收标准
F  代码生成      Programmer/SeniorEngineer  →  patch 或完整文件
G  快速评估      QA            →  [REVIEW] 结构化审查
H  合并检查      静态检查       →  缺失文件/悬空引用
I  总体评估      SeniorEngineer + Architect  →  编译修复 + 架构审查
J  收束归档      ProductOwner  →  更新规则文件、git commit
```

> C～J 在 `'main_loop` 中循环，`[MAJOR]` 问题可回滚至 Phase C（最多 3 次）。
> Phase I 编译修复最多 5 次迭代。

---

## 配置文件 / Configuration

配置文件位置优先级：

1. `./sakichan.conf`（项目级）
2. `~/.config/sakichan/sakichan.conf`（用户级，自动生成）
3. `$SAKICHAN_CONFIG`（环境变量）

示例：

```toml
[general]
lang = "zh_TW"          # zh_TW / en
edit_mode = false

[backend]
type = "ollama"         # ollama（目前可用）/ lmcpp（预留）

[backend.ollama]
host = "http://localhost:11434"

[slots.Architect]
primary = "deepseek-r1:14b"   # 改用更大的模型
fallback = ["deepseek-r1:8b", "qwen2.5-coder:7b"]

[presets.architect]
temperature = 0.4
top_p = 0.95

[[verification]]
detector = { file_exists = "build.zig" }
name = "Zig"
command = "zig"
args = ["build"]
```

---

## CLI 命令 / Commands

| 命令 | 说明 |
|------|------|
| `/help` | 显示帮助 |
| `/slots` | 查看所有插槽当前状态及可用模型 |
| `/slot <插槽> [模型]` | 查看或临时修改插槽模型 |
| `/config` | 显示配置文件路径及关键配置 |
| `/models` | 按插槽分组列出模型 |
| `/edit` | 切换始终执行模式（跳过 Phase E 确认） |
| `/lang <zh\|en>` | 切换界面语言 |
| `/undo [n]` | 回滚 n 个 git checkpoint |
| `/history` | 显示 sakichan 相关的 git 提交 |
| `/diff` | 显示当前 git diff --stat |
| `/usage` | 显示 token 用量统计 |
| `/sessions` | 列出历史会话 |
| `/resume <id>` | 恢复会话上下文 |
| `/exit` | 退出 |

---

## 项目文件结构 / File Layout

```
src/
├── main.rs          REPL 循环、后端初始化、插槽分配
├── config.rs        TOML 配置加载、默认值生成
├── slots.rs         SlotRole 枚举、插槽解析、模型探测
├── handoff.rs       结构化交接数据结构与 prompt 构建
├── orchestrator.rs  十阶段编排主逻辑
├── commands.rs      CLI 命令处理（含 /slots /slot /config）
├── display.rs       颜色常量、Spinner、diff 展示
├── state.rs         AppState（含 config + slot_assignments）
├── executor.rs      跨平台 shell 执行器（含危险命令过滤）
├── logger.rs        build.log 追加写入
├── rules.rs         .sakichan.md 管理
└── backend/
    ├── mod.rs       LlmBackend trait、ModelOptions、create_backend()
    └── ollama.rs    OllamaBackend 实现
```

---

## 持久化 / Persistence

```
.sakichan/
  history.txt          readline 历史
  usage.json           每日 + 累计 token 用量
  sessions/{uuid}.json 可恢复的会话快照
~/.config/sakichan/
  sakichan.conf        用户级配置（首次运行自动生成）
.sakichan.md           项目规则（AI 每次运行读取并更新）
```

---

## 系统标签 / System Tags

AI 响应中可嵌入以下标签，Saki-chan 会自动解析并注入结果：

```
[SYSTEM:read_file path="src/main.rs"]
[SYSTEM:list_files path="src"]
[SYSTEM:grep pattern="fn main" path="src"]
[SYSTEM:web_search query="rust serde deserialize enum"]
```

---

## 安全 / Safety

- `executor.rs` 过滤 `rm -rf /`、`diskpart` 等危险命令
- **Source-file guard**：新内容 < 原文件行数 30% 时拒绝写入
- **Prose guard**：检测到 Markdown 散文试图覆盖源文件时拒绝写入
- **Git checkpoint**：Phase F 前自动 commit，`/undo` 可回滚

---

## License

MIT
