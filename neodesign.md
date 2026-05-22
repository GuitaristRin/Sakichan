# Sakichan v0.4.0 迁移设计文档

## 从双模型硬编码到角色驱动的自适应 AI 团队

---

**文档状态**: 待确认  
**目标版本**: v0.4.0  
**设计原则**: 零用户依赖、角色驱动、结构化交接、硬件自适应

---

## 目录

1. [架构全景](#1-架构全景)
2. [统一配置系统](#2-统一配置系统)
3. [插槽系统](#3-插槽系统)
4. [结构化交接协议](#4-结构化交接协议)
5. [推理后端抽象](#5-推理后端抽象)
6. [llama.cpp 集成方案](#6-llamacpp-集成方案)
7. [Phase I 验证策略改造](#7-phase-i-验证策略改造)
8. [模型预设系统](#8-模型预设系统)
9. [CLI 命令变更](#9-cli-命令变更)
10. [分阶段迁移步骤](#10-分阶段迁移步骤)
11. [关键数据结构汇总](#11-关键数据结构汇总)

---

## 1. 架构全景

### 1.1 从 v0.3.0 到 v0.4.0 的核心变更

| 维度 | v0.3.0 | v0.4.0 |
|------|--------|--------|
| 模型选择 | 固定双模型 (`DSR1` / `QWEN`) | 五大插槽，按角色路由 |
| 配置文件 | 无，参数散落在代码中 | `sakichan.conf` 统一管理 |
| 推理后端 | 仅 Ollama | 默认 `llama.cpp`（通过 `lmcpp`），Ollama 作为可选后端 |
| 用户依赖 | 需安装 Ollama | **零依赖**，单一二进制 + 模型文件 |
| 验证策略 | 硬编码 `cargo check` | 可扩展探测器表，用户可自定义 |
| Phase 交接 | 隐式（上游输出直接拼接为下游 prompt） | 结构化交接单（Handoff），含上下文、约束、验收标准 |
| 预设管理 | 三个硬编码函数 | 配置文件驱动，插槽绑定预设名 |

### 1.2 五大插槽与 Phase 映射

```
Phase A (Summarize)       → ProductOwner
Phase B (Clarification)   → ProductOwner
Phase C (Solution Design) → Architect
Phase D (Direction Check) → Architect
Phase E (Module Plan)     → Architect
Phase F (Code Generation) → Programmer (默认) / SeniorEngineer (复杂模块)
Phase G (Quick Review)    → QA
Phase H (Merge Check)     → QA + 静态工具
Phase I (Overall Eval)    → SeniorEngineer (诊断) + Architect (架构审查)
Phase J (Wrap-up)         → ProductOwner (验收)
```

### 1.3 串行调用模型

系统**一次只加载一个模型到内存**。Phase 之间检查当前加载的模型是否为目标模型，不是则卸载旧模型、加载新模型。

```rust
struct LmCppBackend {
    loaded_model: Mutex<Option<(String, LlamaModel)>>,
    // ...
}

impl LmCppBackend {
    fn ensure_model_loaded(&self, model_name: &str) -> Result<()> {
        let mut guard = self.loaded_model.lock().unwrap();
        match &*guard {
            Some((name, _)) if name == model_name => Ok(()),  // 已加载
            _ => {
                drop(guard);  // 释放锁，卸载旧模型
                let model = self.load_model(model_name)?;
                *self.loaded_model.lock().unwrap() = Some((model_name.to_string(), model));
                Ok(())
            }
        }
    }
}
```

---

## 2. 统一配置系统

### 2.1 文件位置与优先级

```
1. ./sakichan.conf                      # 项目级配置
2. ~/.config/sakichan/sakichan.conf     # 用户级配置
3. $SAKICHAN_CONFIG                     # 环境变量指定路径
```

找到第一个即停止。**首次运行时，静默在用户级目录创建默认配置文件**（不打扰用户）。

### 2.2 完整配置文件结构

```toml
# ============================================================
# Sakichan v0.4.0 统一配置文件
# 此文件在首次运行时自动生成。所有字段可选，未设置的使用默认值。
# ============================================================

# --- 通用设置 ---
[general]
lang = "zh-TW"          # 界面语言: zh-CN / zh-TW / en
edit_mode = false       # true = 跳过 Phase E 后的确认步骤
log_level = "info"      # trace / debug / info / warn / error

# --- 推理后端 ---
[backend]
type = "lmcpp"          # 后端类型: lmcpp (默认) / ollama

# llama.cpp 设置
[backend.lmcpp]
model_dirs = [".", "~/.cache/sakichan/models"]
n_gpu_layers = -1       # -1 = 全加载到 GPU, 0 = 纯 CPU
n_ctx = 8192
n_batch = 512

# Ollama 设置 (仅 backend.type = "ollama" 时生效)
[backend.ollama]
host = "http://localhost:11434"

# --- 团队插槽 ---
[slots.ProductOwner]
primary = "qwen2.5-coder:7b"
fallback = ["deepseek-r1:8b"]
models = ["qwen2.5-coder:7b", "deepseek-r1:8b", "llama3:8b"]

[slots.Architect]
primary = "deepseek-r1:8b"
fallback = ["deepseek-r1:14b", "qwen2.5-coder:7b"]
models = ["deepseek-r1:8b", "deepseek-r1:14b", "deepseek-r1:32b", "qwen2.5-coder:7b"]

[slots.SeniorEngineer]
primary = "deepseek-r1:8b"
fallback = ["qwen2.5-coder:7b"]
models = ["deepseek-r1:8b", "deepseek-r1:14b", "qwen2.5-coder:7b"]

[slots.Programmer]
primary = "qwen2.5-coder:7b"
fallback = ["deepseek-coder:6.7b", "codellama:7b"]
models = ["qwen2.5-coder:7b", "deepseek-coder:6.7b", "codellama:7b", "deepseek-r1:8b"]

[slots.QA]
primary = "qwen2.5-coder:7b"
fallback = ["deepseek-r1:8b"]
models = ["qwen2.5-coder:7b", "deepseek-r1:8b"]

# --- 模型参数预设 ---
[presets]
[presets.default]
temperature = 0.3
top_p = 0.9
top_k = 40
repeat_penalty = 1.1

[presets.architect]
temperature = 0.4
top_p = 0.95
top_k = 50
repeat_penalty = 1.05

[presets.programmer]
temperature = 0.2
top_p = 0.9
top_k = 40
repeat_penalty = 1.1

[presets.reviewer]
temperature = 0.1
top_p = 0.85
top_k = 20
repeat_penalty = 1.2

# --- 插槽→预设绑定 ---
[slot_presets]
ProductOwner = "default"
Architect = "architect"
SeniorEngineer = "default"
Programmer = "programmer"
QA = "reviewer"

# --- Phase I 验证策略 ---
[[verification]]
detector = { file_exists = "Cargo.toml", contains = "[package]" }
name = "Rust"
command = "cargo"
args = ["check"]

[[verification]]
detector = { any_extension = ".py" }
name = "Python"
command = "python"
args = ["-m", "py_compile", "{files}"]

[[verification]]
detector = { all_files = ["package.json", "tsconfig.json"] }
name = "TypeScript"
command = "npx"
args = ["tsc", "--noEmit"]

[[verification]]
detector = { file_exists = "CMakeLists.txt" }
name = "C++ (CMake)"
command = "cmake"
args = ["--build", "build"]

[[verification]]
detector = { always = true }
name = "Manual Review"
# 无 command = 仅架构审查，不执行自动化验证
```

### 2.3 默认配置自动生成

首次运行 `sakichan` 时：

1. 检查 `~/.config/sakichan/sakichan.conf` 是否存在
2. 不存在 → 将嵌入的默认配置写入该路径
3. 打印: `ℹ️  已在 ~/.config/sakichan/sakichan.conf 创建默认配置`
4. 继续正常运行

默认配置内容为上述完整 TOML，`backend.type = "lmcpp"`。

---

## 3. 插槽系统

### 3.1 五大插槽定义

| 插槽 | 角色 | 核心职责 | 典型 Phase |
|------|------|---------|-----------|
| `ProductOwner` | 产品负责人 | 需求总结、澄清提问、验收评估 | A, B, J |
| `Architect` | 系统架构师 | 方案设计、模块拆分、方向决策、架构审查 | C, D, E, I |
| `SeniorEngineer` | 高级工程师 | 复杂逻辑实现、编译诊断、跨模块修复 | F(复杂模块), I |
| `Programmer` | 程序员 | 具体模块代码生成、遵循规范 | F(默认) |
| `QA` | 质量保证 | 代码审查、静态检查 | G, H |

### 3.2 插槽解析算法 (`resolve_slot`)

```rust
fn resolve_slot(config: &SlotConfig, available: &HashSet<String>) -> Result<String> {
    // 1. 检查 primary 是否可用
    if available.contains(&config.primary) {
        return Ok(config.primary.clone());
    }
    // 2. 遍历 fallback，返回第一个可用的
    for model in &config.fallback {
        if available.contains(model) {
            return Ok(model.clone());
        }
    }
    // 3. 遍历 models，返回第一个可用的非 primary 模型
    for model in &config.models {
        if model != &config.primary && available.contains(model) {
            return Ok(model.clone());
        }
    }
    // 4. 全部不可用
    Err(anyhow!("插槽无可用模型。请检查配置或下载模型文件。"))
}
```

### 3.3 模型可用性探测

启动时自动扫描 `model_dirs`，记录所有找到的 `.gguf` 文件：

```rust
fn probe_available_models(model_dirs: &[PathBuf]) -> HashSet<String> {
    let mut available = HashSet::new();
    for dir in model_dirs {
        if let Ok(entries) = std::fs::read_dir(expand_path(dir)) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "gguf") {
                    if let Some(stem) = path.file_stem() {
                        available.insert(stem.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    available
}
```

### 3.4 AI 架构师的模型选择边界

AI（Phase E 制定模块计划时）可为模块指定 `assigned_slot`：

```text
[MODULE: auth]
assigned_slot: SeniorEngineer    # 合法值: ProductOwner / Architect / SeniorEngineer / Programmer / QA
```

系统在 Phase F 将 `assigned_slot` 传入 `resolve_slot()` 解析为具体模型名。AI **不能**直接指定具体模型名，只能从五个插槽中选择。

---

## 4. 结构化交接协议

### 4.1 设计原理

每个 Phase 的交接点都是**错误累积的风险点**。结构化交接通过强制书面交接、明确约束条件、可验证的验收标准，模拟真实团队中"拒绝口头传话"的最佳实践。

### 4.2 交接数据结构

```rust
/// 通用的结构化交接单
struct Handoff<T> {
    /// 上游角色
    from: SlotRole,
    /// 下游角色
    to: SlotRole,
    /// 上游已知的关键上下文
    context: HandoffContext,
    /// 具体交付物
    artifact: T,
    /// 下游必须遵守的约束
    constraints: Vec<Constraint>,
    /// 下游如何验证自己是否正确
    acceptance_criteria: Vec<String>,
    /// 时间戳
    timestamp: chrono::DateTime<chrono::Utc>,
}

struct HandoffContext {
    /// 之前所有阶段的关键决策
    decisions: Vec<String>,
    /// 不可变的假设前提
    assumptions: Vec<String>,
    /// 已知风险
    known_risks: Vec<String>,
}

enum Constraint {
    /// 硬性约束：不可违反
    Hard(String),
    /// 软性约束：违反前需说明理由
    Soft(String),
    /// 信息提示：仅供参考
    Info(String),
}
```

### 4.3 关键交接点定义

#### 交接点 1: Phase E → Phase F (Architect → Programmer)

**风险**: 程序员误解架构意图，生成不符合规格的代码。

**交接内容**:

```text
[MODULE: auth]
inputs:
  - 用户凭据 (email: String, password: String)
  - 数据库连接池 (来自 db 模块的 DbPool)
outputs:
  - AuthToken: struct { user_id, token_hash, expires_at }
  - pub fn verify_token(pool: &DbPool, token: &str) -> Result<AuthToken>
constraints:
  - [HARD] 密码必须用 argon2 哈希，不可明文存储
  - [HARD] 不要修改 db 模块的公开接口
  - [HARD] 错误类型使用项目统一的 crate::Error，不要引入新的错误枚举
  - [SOFT] verify_token() 建议使用缓存减少数据库查询
verification:
  - verify_token(valid_token) 返回 Ok(AuthToken)
  - verify_token(expired_token) 返回 Err(Error::AuthExpired)
  - verify_token(invalid_token) 返回 Err(Error::AuthInvalid)
assigned_slot: Programmer
```

**交接格式**: Phase E 产出 `Vec<Module>`，每个 `Module` 包含上述结构。Phase F 的 prompt 由 `build_generation_prompt(handoff)` 构建，**不是**原始模块计划的直接拼接，而是经过格式化的交接单。

#### 交接点 2: Phase G → Phase F (QA → Programmer)

**风险**: 审查意见模糊，程序员无法准确修复。

**交接内容**:

```text
[REVIEW: auth]
issues:
  - [MAJOR] L42: 变量名 `x` 不清晰，改为 `token_expiry`
  - [MAJOR] verify_token() 未处理 Token 过期情况
  - [MINOR] hash_password() 缺少单元测试
  - [INFO] 建议将 argon2 参数提取为常量
verification:
  - [ ] L42 变量名已修改
  - [ ] verify_token() 包含过期检查逻辑
  - [ ] 新增 hash_password() 的单元测试
status: REVISION_REQUIRED
```

**交接格式**: Phase G 产出 `ReviewResult`，包含结构化的问题列表和验证清单。Phase F 的修复 prompt 由 `build_fix_prompt(original_code, review_result)` 构建。

#### 交接点 3: Phase I [MAJOR] → Phase C (回滚)

**风险**: 架构师重新设计时不了解失败原因，重蹈覆辙。

**交接内容**:

```text
[MAJOR_RESTART]
triggered_by: SeniorEngineer (Phase I 架构审查)
reason: "模块 auth 和 db 存在循环依赖。auth 需要 DbPool，db 审计需要 AuthToken。"
attempted_fixes:
  - "延迟初始化注入 → 编译失败 (E0502)"
  - "回调注册模式 → 运行时死锁"
  - "引入 Arc<RwLock<Option<DbPool>>> → 编译通过但架构评审认为过度工程化"
constraints_for_redesign:
  - [HARD] 必须打破 auth ↔ db 的循环依赖
  - [HARD] 不要改变已有的 Error 类型定义
  - [SOFT] 可考虑引入中间层 (如 EventBus)
preserved_artifacts:
  - "models.rs (用户模型定义) 保持不变"
  - "error.rs 保持不变"
```

**交接格式**: Phase I 产出 `MajorRestartReport`，Phase C 重启时作为 `HandoffContext` 的一部分注入 prompt。

### 4.4 交接验证步骤（接收方复述确认）

在每个跨模型交接发生后，接收方在开始工作前需复述理解：

```
System Prompt 注入:
"你是团队中的 {接收方角色}。{上游角色} 交付了以下交接单。
在开始工作前，请用 2-3 句话复述:
1. 你要交付什么？
2. 最重要的约束是什么？
3. 如何验证你的产出是正确的？"
```

复述结果与原始交接单做相似度比对。如果偏差超过阈值（实测确定），触发澄清流程：将复述和原始交接单一并送回上游，请求澄清。

---

## 5. 推理后端抽象

### 5.1 Trait 定义

```rust
/// 推理后端统一接口
trait LlmBackend: Send + Sync {
    /// 流式生成
    fn generate_stream(
        &self,
        model: &str,
        prompt: &str,
        options: &ModelOptions,
    ) -> Result<Box<dyn Iterator<Item = Result<String>> + '_>>;

    /// 非流式生成（用于需要完整响应的场景，如 JSON 解析）
    fn generate_complete(
        &self,
        model: &str,
        prompt: &str,
        options: &ModelOptions,
    ) -> Result<String> {
        let mut full = String::new();
        for token in self.generate_stream(model, prompt, options)? {
            full.push_str(&token?);
        }
        Ok(full)
    }

    /// 列出已加载/可用的模型
    fn list_models(&self) -> Vec<String>;

    /// 后端名称（用于日志和显示）
    fn backend_name(&self) -> &'static str;
}

/// 模型推理参数
struct ModelOptions {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub repeat_penalty: f32,
    pub max_tokens: Option<u32>,
}

/// 后端工厂：根据配置创建后端实例
fn create_backend(config: &SakichanConfig) -> Result<Box<dyn LlmBackend>> {
    match config.backend.r#type.as_str() {
        "ollama" => Ok(Box::new(OllamaBackend::new(&config.backend.ollama)?)),
        "lmcpp" => Ok(Box::new(LmCppBackend::new(&config.backend.lmcpp)?)),
        other => Err(anyhow!("不支持的后端类型: {}", other)),
    }
}
```

### 5.2 后端实现策略

| 后端 | 文件 | 实现方式 |
|------|------|---------|
| `LmCppBackend` | `src/backend/lmcpp.rs` | 基于 `lmcpp` crate，维护 `Mutex<Option<(String, LlamaModel)>>` |
| `OllamaBackend` | `src/backend/ollama.rs` | 基于现有 `ollama.rs` 的 HTTP 调用逻辑，封装为 trait 实现 |

### 5.3 核心逻辑：`call_model` 重构

现有 `call_model()` 在 `orchestrator.rs` 中直接调用 Ollama API。重构后：

```rust
/// 统一的模型调用入口
fn call_model(
    backend: &dyn LlmBackend,
    slot: SlotRole,
    prompt: &str,
    state: &AppState,
) -> Result<String> {
    let model_name = state.slot_assignments.get(&slot)
        .ok_or_else(|| anyhow!("插槽 {:?} 未分配模型", slot))?;
    let preset = state.config.get_preset_for_slot(slot);
    let options = ModelOptions::from_preset(preset);

    // 系统环境头保持你现有的 Windows 11 / PowerShell 声明
    let full_prompt = format!("{}\n\n{}", ENVIRONMENT_HEADER, prompt);

    let mut response = String::new();
    for token in backend.generate_stream(model_name, &full_prompt, &options)? {
        let token = token?;
        response.push_str(&token);
        // 原有的 Spinner 更新逻辑
    }

    // 原有的 process_system_calls 逻辑（与后端无关）
    process_system_calls(&mut response, state)?;

    Ok(response)
}
```

`call_model` **不**处理结构化交接的 prompt 格式化——由各个 Phase 的函数在调用前准备好完整的 prompt。交接单的构建和格式化逻辑在 `src/handoff.rs` 中。

---

## 6. llama.cpp 集成方案

### 6.1 依赖库选择：`lmcpp`

**选择理由**:

- 自动化工具链：`lmcpp` 在 `build.rs` 中自动下载和编译 `llama.cpp` 的 C/C++ 源码，开发者无需手动安装 `cmake` 或配置 C++ 编译环境
- API 封装友好：提供 `LlamaModel`、`LlamaSession`、`apply_chat_template` 等高层抽象
- 对话模板支持：能自动读取 GGUF 文件中的 `tokenizer.chat_template`，Qwen2.5 和 DeepSeek-R1 的模板格式差异由库处理
- 活跃维护：与 `llama.cpp` 上游保持同步

### 6.2 Cargo.toml 配置

```toml
[features]
default = ["lmcpp"]
lmcpp = ["dep:lmcpp"]
ollama = ["dep:reqwest"]

[dependencies]
lmcpp = { version = "0.8", optional = true }
reqwest = { version = "0.12", features = ["blocking", "json"], optional = true }
# ... 其他依赖

[build-dependencies]
# lmcpp 的 build.rs 会自动处理 llama.cpp 的编译
```

`cargo build --release` 默认启用 `lmcpp` feature，编译出零依赖的独立二进制文件。

### 6.3 LmCppBackend 实现要点

```rust
// src/backend/lmcpp.rs

use lmcpp::{LlamaModel, LlamaSession, ModelParams, SessionParams, ChatMessage};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct LmCppBackend {
    loaded_model: Mutex<Option<(String, LlamaModel)>>,
    model_dirs: Vec<PathBuf>,
    available_models: HashSet<String>,
    n_gpu_layers: i32,
    n_ctx: u32,
    n_batch: u32,
}

impl LmCppBackend {
    pub fn new(config: &LmCppConfig) -> Result<Self> {
        let available = probe_gguf_files(&config.model_dirs);
        Ok(Self {
            loaded_model: Mutex::new(None),
            model_dirs: config.model_dirs.clone(),
            available_models: available,
            n_gpu_layers: config.n_gpu_layers,
            n_ctx: config.n_ctx,
            n_batch: config.n_batch,
        })
    }

    fn find_model_path(&self, model_name: &str) -> Result<PathBuf> {
        for dir in &self.model_dirs {
            let path = expand_path(dir).join(format!("{}.gguf", model_name));
            if path.exists() {
                return Ok(path);
            }
        }
        Err(anyhow!(
            "找不到模型文件: {}.gguf\n搜索路径: {:?}\n请从 HuggingFace 下载 GGUF 格式的模型文件。",
            model_name, self.model_dirs
        ))
    }

    fn load_model(&self, model_name: &str) -> Result<LlamaModel> {
        let path = self.find_model_path(model_name)?;
        let params = ModelParams::default()
            .n_gpu_layers(self.n_gpu_layers)
            .n_ctx(self.n_ctx)
            .n_batch(self.n_batch);
        LlamaModel::from_file(path, params)
            .map_err(|e| anyhow!("加载模型 {} 失败: {}", model_name, e))
    }
}

impl LlmBackend for LmCppBackend {
    fn generate_stream(
        &self,
        model_name: &str,
        prompt: &str,
        options: &ModelOptions,
    ) -> Result<Box<dyn Iterator<Item = Result<String>> + '_>> {
        // 1. 确保目标模型已加载（必要时切换）
        self.ensure_model_loaded(model_name)?;
        let guard = self.loaded_model.lock().unwrap();
        let (_, model) = guard.as_ref().unwrap();

        // 2. 构建消息（使用 ChatMessage 让 lmcpp 内部处理模板）
        let messages = vec![ChatMessage::user(prompt)];
        let session_params = SessionParams::default()
            .temperature(options.temperature)
            .top_p(options.top_p)
            .top_k(options.top_k);
        let session = model.create_session(session_params)?;

        // 3. 流式推理
        // lmcpp 的流式 API 返回的是回调式的，这里需要适配为 Iterator
        // 具体实现取决于 lmcpp 的实际 API
        todo!("适配 lmcpp 流式 API")
    }

    fn list_models(&self) -> Vec<String> {
        self.available_models.iter().cloned().collect()
    }

    fn backend_name(&self) -> &'static str {
        "llama.cpp (lmcpp)"
    }
}
```

### 6.4 模型切换开销

在串行架构下，模型切换只发生在 Phase 之间。卸载旧模型 + 加载新模型预估耗时：

| 模型规模 | 加载时间 (NVMe SSD) |
|---------|-------------------|
| 7B Q4_K_M (~4.5GB) | 2-4 秒 |
| 8B Q4_K_M (~5GB) | 3-5 秒 |
| 14B Q4_K_M (~9GB) | 6-10 秒 |

对于天选 2 (RTX 3070 8G)，7B/8B 模型可以全部加载到 GPU (`n_gpu_layers = -1`)，加载时间在可接受范围内。

---

## 7. Phase I 验证策略改造

### 7.1 问题

当前 Phase I 硬编码 `cargo check`，无论什么项目类型都执行。这导致写论文、Python 脚本时，系统仍尝试运行 Rust 编译器。

### 7.2 解决方案：可扩展验证策略表

验证策略从配置文件加载，支持用户扩展：

```toml
[[verification]]
detector = { file_exists = "Cargo.toml", contains = "[package]" }
name = "Rust"
command = "cargo"
args = ["check"]

[[verification]]
detector = { always = true }
name = "Manual Review"
# 无 command = 仅架构审查
```

### 7.3 探测器类型

```rust
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Detector {
    /// 特定文件存在且包含指定内容
    FileExists {
        file_exists: String,
        #[serde(default)]
        contains: Option<String>,
    },
    /// 存在特定扩展名的文件
    AnyExtension {
        any_extension: String,
    },
    /// 多个文件全部存在
    AllFiles {
        all_files: Vec<String>,
    },
    /// 始终匹配（用于兜底策略）
    Always {
        always: bool,
    },
}

impl Detector {
    fn matches(&self, project_root: &Path) -> bool {
        match self {
            Detector::FileExists { file_exists, contains } => {
                let path = project_root.join(file_exists);
                if !path.exists() { return false; }
                if let Some(needle) = contains {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        return content.contains(needle.as_str());
                    }
                    return false;
                }
                true
            }
            Detector::AnyExtension { any_extension } => {
                has_files_with_extension(project_root, any_extension)
            }
            Detector::AllFiles { all_files } => {
                all_files.iter().all(|f| project_root.join(f).exists())
            }
            Detector::Always { .. } => true,
        }
    }
}
```

### 7.4 Phase I 新逻辑

```rust
fn phase_i_overall_eval(state: &mut AppState, backend: &dyn LlmBackend) -> Result<()> {
    // 1. 从配置表中匹配验证策略
    let strategy = state.config.verification.iter()
        .find(|v| v.detector.matches(&state.work_dir))
        .unwrap();  // 兜底策略 always 保证一定能匹配到

    // 2. 显示验证策略
    println!("🔍 Phase I: 整体评估 (验证策略: {})", strategy.name);

    // 3. 始终执行架构审查 (Architect 插槽)
    run_architect_review(state, backend)?;

    // 4. 如果有编译命令，执行编译检查循环
    if let Some(cmd) = &strategy.command {
        let files = collect_project_files(&state.work_dir, &strategy);
        run_compile_check_loop(state, cmd, &strategy.args, &files, backend)?;
    } else {
        println!("ℹ️  此项目类型仅进行架构审查，跳过自动化验证");
    }

    // 5. 处理架构审查结果（[MAJOR] 触发重启等原有逻辑）
    handle_architect_review_result(state)
}
```

### 7.5 用户扩展验证策略示例

用户可在 `sakichan.conf` 中添加自定义策略：

```toml
# Zig 项目
[[verification]]
detector = { file_exists = "build.zig" }
name = "Zig"
command = "zig"
args = ["build"]

# Haskell (Stack)
[[verification]]
detector = { file_exists = "stack.yaml" }
name = "Haskell (Stack)"
command = "stack"
args = ["build"]
```

新增策略只需编辑配置文件，无需修改代码。

---

## 8. 模型预设系统

### 8.1 设计

预设定义与插槽解耦。插槽通过 `slot_presets` 表绑定预设名：

```toml
[presets.programmer]
temperature = 0.2
top_p = 0.9
top_k = 40

[slot_presets]
Programmer = "programmer"
```

用户可以为某个插槽单独指定预设，或在插槽定义中直接内联参数：

```toml
# 方式一：引用预定义预设
[slot_presets]
Programmer = "programmer"

# 方式二：直接内联（覆盖预定义预设）
[slots.Programmer]
primary = "qwen2.5-coder:7b"
preset_override = { temperature = 0.15, top_p = 0.85 }  # 可选，覆盖预设参数
```

### 8.2 四种内置预设

| 预设名 | 适用场景 | temperature | 特点 |
|--------|---------|------------|------|
| `default` | 通用推理 | 0.3 | 平衡创造性与确定性 |
| `architect` | 方案设计、模块规划 | 0.4 | 稍高创造性，利于发散思考 |
| `programmer` | 代码生成 | 0.2 | 高确定性，减少语法错误 |
| `reviewer` | 代码审查、验收 | 0.1 | 极低温度，确保审查一致性 |

### 8.3 预设解析逻辑

```rust
impl SakichanConfig {
    fn get_preset_for_slot(&self, slot: SlotRole) -> ModelPreset {
        // 1. 检查 slot_presets 绑定
        let preset_name = self.slot_presets.get(&slot);
        let preset = preset_name
            .and_then(|name| self.presets.get(name))
            .unwrap_or(&self.presets.default);

        // 2. 检查插槽的内联覆盖
        if let Some(slot_config) = self.slots.get(&slot) {
            if let Some(override_preset) = &slot_config.preset_override {
                return preset.merge(override_preset);
            }
        }

        preset.clone()
    }
}
```

---

## 9. CLI 命令变更

### 9.1 变更总览

| 命令 | v0.3.0 | v0.4.0 | 说明 |
|------|--------|--------|------|
| `/help` | 保留 | 保留 | 更新帮助文本 |
| `/models` | 列出所有模型 | 按插槽分组显示 | 显示每个插槽的主模型和可用备选 |
| `/model <name>` | 切换模型 | **废弃** | 替换为 `/slot` |
| `/slot <slot> [model]` | 无 | **新增** | 修改指定插槽的主模型 |
| `/slots` | 无 | **新增** | 显示所有插槽的当前配置 |
| `/config` | 无 | **新增** | 显示配置文件路径和当前关键配置 |
| `/clear` | 保留 | 保留 | |
| `/init` | 创建 .sakichan.md | 同时生成 sakichan.conf.example | |
| `/load <file>` | 保留 | 保留 | |
| `/usage` | 保留 | 保留 | |
| `/sessions` | 保留 | 保留 | |
| `/resume <id>` | 保留 | 保留 | |
| `/export <id>` | 保留 | 保留 | |
| `/edit` | 切换 edit_mode | 保留 | 同时写回配置文件 |
| `/lang <zh\|en>` | 切换语言 | 保留 | 同时写回配置文件 |
| `/undo [n]` | 保留 | 保留 | |
| `/history` | 保留 | 保留 | |
| `/diff` | 保留 | 保留 | |
| `/exit` | 保留 | 保留 | |

### 9.2 新增命令详情

#### `/slots`

显示所有插槽的当前状态：

```
══════════════════════════════════════════
  团队插槽状态
──────────────────────────────────────────
  ProductOwner    → qwen2.5-coder:7b      [可用: qwen2.5-coder:7b, deepseek-r1:8b]
  Architect       → deepseek-r1:8b        [可用: deepseek-r1:8b]
  SeniorEngineer  → deepseek-r1:8b        [可用: deepseek-r1:8b, qwen2.5-coder:7b]
  Programmer      → qwen2.5-coder:7b      [可用: qwen2.5-coder:7b]
  QA              → qwen2.5-coder:7b      [可用: qwen2.5-coder:7b, deepseek-r1:8b]
──────────────────────────────────────────
  后端: llama.cpp (lmcpp) | 已加载: qwen2.5-coder:7b
══════════════════════════════════════════
```

#### `/slot <slot> [model]`

修改指定插槽的主模型：

```
> /slot Architect deepseek-r1:14b
✓ Architect 插槽主模型已切换为 deepseek-r1:14b
⚠ 此模型当前未在本地找到。下次调用时将尝试加载。
```

不带 `model` 参数时，显示该插槽的详细配置和可用模型列表。

#### `/config`

显示当前配置：

```
══════════════════════════════════════════
  当前配置
──────────────────────────────────────────
  配置文件: ~/.config/sakichan/sakichan.conf
  后端: llama.cpp (lmcpp)
  语言: zh-TW
  编辑模式: 关闭
  模型目录: ., ~/.cache/sakichan/models
══════════════════════════════════════════
```

---

## 10. 分阶段迁移步骤

### 阶段 1: 基础设施 (预计新增约 800 行)

**目标**: 新系统可编译运行，但不影响现有功能。

1. 创建 `src/config.rs` — 配置加载、默认配置生成
2. 创建 `src/slots.rs` — 插槽系统核心逻辑 (`resolve_slot`, `probe_available_models`)
3. 创建 `src/handoff.rs` — 交接数据结构定义、`build_*_prompt` 函数
4. 创建 `src/backend/mod.rs` — `LlmBackend` trait 定义、`ModelOptions`
5. 创建 `src/backend/lmcpp.rs` — `LmCppBackend` 空壳实现
6. 修改 `Cargo.toml` — 添加 `lmcpp` 依赖和 feature flags

**验证**: `cargo build` 通过，新模块可导入但不影响现有行为。

### 阶段 2: 后端切换 (预计新增约 400 行)

**目标**: 在 `lmcpp` 后端上成功运行一次完整推理。

1. 完善 `LmCppBackend::new()` — 模型探测、首次加载
2. 实现 `LmCppBackend::generate_stream()` — 流式 token 生成
3. 实现模型切换逻辑 (`ensure_model_loaded`)
4. 在 `main.rs` 中集成 `create_backend()`
5. 手动测试：用 `sakichan` 发送一次简单 prompt，验证流式输出正确

**验证**: 用默认配置启动，完成一次 Phase A → Phase F 的简单任务（如生成一个 `hello.rs`）。

### 阶段 3: 插槽系统集成 (预计修改约 300 行)

**目标**: 将硬编码的双模型替换为插槽驱动的模型选择。

1. 在 `AppState` 中新增 `config: SakichanConfig`, `slot_assignments: HashMap<SlotRole, String>`
2. 重构 `orchestrator.rs` 中所有 Phase 函数：`resolve_model()` → `resolve_slot()`
3. 移除 `DSR1` / `QWEN` 常量，改为 `SlotRole` 枚举
4. 修改 `/model` → `/slot` 命令
5. 修改 `/models` → 按插槽分组显示

**验证**: 使用默认配置完成一次完整十阶段任务。`/slots` 命令正确显示插槽状态。

### 阶段 4: 交接协议实现 (预计新增约 500 行)

**目标**: 在关键交接点使用结构化交接单。

1. 修改 Phase E 输出解析：`parse_modules()` 支持 `constraints`, `verification` 字段
2. 新增 `build_handoff_prompt()` — 将交接单转换为下游 prompt
3. 修改 Phase F：使用交接单中的 constraints 构建 prompt
4. 修改 Phase G：产出结构化的 `ReviewResult` 而不是自由文本
5. 修改 Phase I [MAJOR] 处理：产出 `MajorRestartReport`
6. 实现交接验证步骤（接收方复述确认）

**验证**: 故意制造一个复杂任务（如"实现一个带有循环依赖风险的模块"），观察 [MAJOR] 回滚时的交接报告是否包含足够信息。

### 阶段 5: Phase I 验证策略改造 (预计修改约 200 行)

**目标**: 验证策略由配置驱动。

1. 实现 `Detector::matches()` 方法
2. 重构 `phase_i_overall_eval()` 使用策略表
3. 移除硬编码的 `cargo check` 逻辑
4. 测试：对非 Rust 项目（如纯 Python 项目）启动，验证跳过 `cargo check`

**验证**: 在一个 Python 项目中运行 `sakichan`，确认 Phase I 不执行 `cargo check`。

### 阶段 6: 清理与文档 (预计删除约 200 行)

**目标**: 移除遗留代码，完善文档。

1. 删除 `src/ollama.rs`（Ollama 后端移至 `src/backend/ollama.rs`）
2. 删除硬编码的 `DSR1` / `QWEN` 常量
3. 删除硬编码的 `dsr1_opts()` / `qwen_ctx_opts()` / `qwen_gen_opts()` 函数
4. 更新 `CLAUDE.md` 反映新架构
5. 更新 README（用户文档）

**验证**: `cargo build --release` 生成独立二进制文件。在新机器上（无 Ollama、无 Python）解压后放入模型文件，运行完整十阶段任务。

---

## 11. 关键数据结构汇总

### 11.1 枚举定义

```rust
/// 五个团队角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
enum SlotRole {
    ProductOwner,
    Architect,
    SeniorEngineer,
    Programmer,
    QA,
}

/// 十阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    A, B, C, D, E, F, G, H, I, J,
}

/// Phase 内部状态
#[derive(Debug, Clone, Serialize, Deserialize)]
enum PhaseState {
    NotStarted,
    Summarizing,
    WaitingForClarification { questions_asked: u8 },
    Designing,
    WaitingForDirection,
    Planning,
    GeneratingModule { module_index: usize, attempt: u8 },
    ReviewingModule { module_index: usize, attempt: u8 },
    FixingModule { module_index: usize, attempt: u8 },
    MergingModules,
    Compiling { attempt: u8 },
    ArchitectReview,
    FixingCompileError { attempt: u8 },
    WrappingUp,
    Completed,
    Failed { reason: String },
}
```

### 11.2 核心结构体

```rust
/// 顶层配置
struct SakichanConfig {
    general: GeneralConfig,
    backend: BackendConfig,
    slots: HashMap<SlotRole, SlotConfig>,
    presets: HashMap<String, ModelPreset>,
    slot_presets: HashMap<SlotRole, String>,
    verification: Vec<VerificationStrategy>,
}

struct SlotConfig {
    primary: String,
    fallback: Vec<String>,
    models: Vec<String>,
    preset_override: Option<ModelPreset>,
}

struct ModelPreset {
    temperature: f32,
    top_p: f32,
    top_k: u32,
    repeat_penalty: f32,
    max_tokens: Option<u32>,
}

struct ModelOptions {
    temperature: f32,
    top_p: f32,
    top_k: u32,
    repeat_penalty: f32,
    max_tokens: Option<u32>,
}

/// 结构化交接单
struct Handoff<T> {
    from: SlotRole,
    to: SlotRole,
    context: HandoffContext,
    artifact: T,
    constraints: Vec<Constraint>,
    acceptance_criteria: Vec<String>,
    timestamp: DateTime<Utc>,
}

struct HandoffContext {
    decisions: Vec<String>,
    assumptions: Vec<String>,
    known_risks: Vec<String>,
}

enum Constraint {
    Hard(String),
    Soft(String),
    Info(String),
}

/// 模块定义 (Phase E 产出)
struct Module {
    name: String,
    inputs: Vec<String>,
    outputs: Vec<String>,
    constraints: Vec<Constraint>,
    verification: Vec<String>,
    assigned_slot: SlotRole,
    needs_compile: bool,
}

/// 审查结果 (Phase G 产出)
struct ReviewResult {
    module_name: String,
    issues: Vec<Issue>,
    verification_checklist: Vec<String>,
    status: ReviewStatus,
}

enum ReviewStatus {
    Approved,
    RevisionRequired,
    Rejected,
}

struct Issue {
    severity: IssueSeverity,  // MAJOR / MINOR / INFO
    location: Option<String>,  // 文件名:行号
    description: String,
}

/// 回滚报告 (Phase I [MAJOR] 产出)
struct MajorRestartReport {
    triggered_by: SlotRole,
    reason: String,
    attempted_fixes: Vec<String>,
    constraints_for_redesign: Vec<Constraint>,
    preserved_artifacts: Vec<String>,
}

/// 验证策略
struct VerificationStrategy {
    detector: Detector,
    name: String,
    command: Option<String>,
    args: Vec<String>,
}

/// 应用状态
struct AppState {
    config: SakichanConfig,
    work_dir: PathBuf,
    lang: String,
    edit_mode: bool,
    slot_assignments: HashMap<SlotRole, String>,
    token_usage: TokenUsage,
    current_session: Option<SessionContext>,
    // ... 其他字段
}
```

---

## 附录 A: 模块文件结构 (v0.4.0)

```
src/
├── main.rs                 # REPL 循环、启动初始化
├── commands.rs             # CLI 命令（含新增的 /slot, /slots, /config）
├── display.rs              # 颜色常量、Spinner、diff 展示
├── orchestrator.rs         # 十阶段编排（重构为使用插槽和交接协议）
├── config.rs               # [新增] 配置加载、默认配置生成
├── slots.rs                # [新增] 插槽解析、模型探测
├── handoff.rs              # [新增] 交接数据结构、prompt 构建函数
├── backend/
│   ├── mod.rs              # [新增] LlmBackend trait、ModelOptions、create_backend
│   ├── lmcpp.rs            # [新增] LmCppBackend 实现
│   └── ollama.rs           # [从 ollama.rs 迁移] OllamaBackend 实现
├── state.rs                # AppState、SessionContext、JSON 持久化
├── executor.rs             # 跨平台 shell 执行器
├── logger.rs               # build.log 追加写入
└── rules.rs                # .sakichan.md 管理
```

---

## 附录 B: 与 v0.3.0 的兼容性说明

1.  **Ollama 后端保留**：通过 `backend.type = "ollama"` 可切换回 Ollama，但不再作为默认后端。
2.  **双模型常量废弃**：`DSR1` 和 `QWEN` 常量不再存在。现有代码中的引用需迁移到 `resolve_slot(SlotRole::Architect)` 等调用。
3.  **配置文件生成**：首次运行时自动生成，不会覆盖用户已有的 `.sakichan/` 目录下的数据文件。
4.  **Session 文件格式**：`sessions/{uuid}.json` 需要增加 `slot_assignments` 和 `phase_state` 字段。旧格式的 session 文件需迁移（或标记为不可恢复）。
