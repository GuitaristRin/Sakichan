# 🌸 Sakichan SE

> opencode 编码 agent 的秘书前端 / The secretary frontend for your opencode coding agent

Sakichan SE 是原生 Android 应用。用户在手机上跟"秘书"对话,秘书理解意图后,通过
局域网里的 opencode server 在 PC 上派活、确认权限、总结结果。**手机是遥控器,PC 是
工作台,秘书是翻译官。**

```
┌─────────────┐     自然语言      ┌──────────────┐    结构化指令    ┌──────────────────┐
│  用户(手机)  │ ──────────────► │  秘书 LLM     │ ──────────────► │  opencode server  │
│  Sakichan SE │ ◄────────────── │  DeepSeek V4F │ ◄────────────── │  (PC, headless)   │
└─────────────┘    流式回复+SSE   └──────────────┘   事件流+SSE     └──────────────────┘
```

---

## 快速开始 / Quick Start

**PC 端**(需要安装 opencode):

```bash
opencode serve --hostname 0.0.0.0 --port 4096 --mdns
```

- `--mdns` 让手机能在局域网自动发现这台机器;不带则需手动输 IP。

**手机端**:

1. 构建安装:`./gradlew :app:assembleDebug`
2. 启动 App → 连接页自动扫描附近 opencode 机器
3. 选一台连接(或手动输 `http://<PC-IP>:4096`)
4. 在设置页填入 SenseNova API key + 秘书模型 ID(`deepseek-v4-flash`)
5. 开始对话,让秘书派活给 opencode

---

## 核心角色 / Roles

| 角色 | 模型 / 引擎 | 用途 |
|---|---|---|
| 秘书主脑 | DeepSeek V4 Flash (`deepseek-v4-flash`) | 理解意图、生成 `run_opencode_task` 指令、总结 opencode 输出。`reasoning_effort:"medium"`,思考块保留展示 |
| 图像理解 | SenseNova Flash-Lite | (规划)两阶段管线:Flash-Lite 描述图片 → DeepSeek 回答 |
| 编码执行 | opencode server | 实际编码 agent,读写文件、跑命令、返回 diff |
| 权限批准 | 用户(手机上) | opencode 请求改文件/跑命令时,手机弹出 approve / deny |

---

## 特性 / Features

- **mDNS 自动发现** — 启动即扫描局域网内的 opencode 机器,多机器列表,跨平台
  (Windows / Linux / macOS 均走标准 mDNS 协议)
- **机器 · 项目 · session 树** — 抽屉按「机器 → 项目目录 → session」组织,随时切换、
  新建会话
- **秘书 function calling** — 秘书调用 `run_opencode_task`,不依赖脆弱的文本标记
- **实时事件流** — opencode 的 token 输出、工具调用、权限请求经 SSE 实时上屏
- **权限回调** — 手机上一键 approve / deny / 总是允许,回传 opencode
- **Metro 直角 UI** — 基于 Kanesumi UI 库,零 Material 3,墨绿主题
- **加密存储** — API key 用 EncryptedSharedPreferences 加密保存

---

## 架构 / Architecture

```
app (Android, Kotlin + Compose)
├── ui/
│   ├── connection/   启动连接页:NsdManager 扫描 + 手动输入
│   ├── chat/         聊天页 + session 树抽屉 + 权限条
│   └── settings/     API key / 模型配置
├── data/
│   ├── network/      OpencodeClient(HTTP+SSE)+ ChatApiClient(LLM 流式)
│   ├── discovery/    mDNS 扫描(_http._tcp 过滤 opencode-*)
│   └── repository/   AppConfigRepository + SecretaryPrompt
├── core/
│   ├── model/        opencode API 模型 + SessionTree
│   └── session/      滑动窗口上下文
└── connection/       ConnectionManager(活跃机器状态)
```

**Session 树**:`机器 id → 项目目录名 → session id`,多机器各管各的。

**UI 组件**:`MetroTextField` / `MetroDrawer` / `MetroChatInputBar` 实战自建,
已贡献回 [Kanesumi](https://github.com/GuitaristRin/Kanesumi)。

---

## 技术栈 / Tech Stack

| 层 | 选型 |
|---|---|
| UI | Kanesumi(includeBuild 组合构建接入),主题色 `#1C5035` 墨绿 |
| 语言 | Kotlin 1.9.24 / AGP 8.5.0 / Compose BOM 2024.12.01 |
| 网络 | OkHttp 4.12 + okhttp-sse(SSE 事件流) |
| DI | Koin 4.0.2 |
| 序列化 | kotlinx-serialization 1.6.3 |
| 存储 | DataStore + EncryptedSharedPreferences |
| 构建 | gradle 9.3.1,compileSdk 36 / minSdk 26 |

---

## 构建 / Build

```bash
./gradlew :app:assembleDebug      # debug APK → app/build/outputs/apk/debug/
./gradlew assemble                # 全量构建
```

---

## 开发文档

构建 / 迁移 / 架构决策详见 [`BUILD.md`](BUILD.md)。

## License

[GPL-3.0](LICENSE)
