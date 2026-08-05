# BUILD.md — Sakichan SE 构建指南

> 维护于 2026-08-04,项目已迁移至 `/home/rain/projects/Sakichan`。

## 1. 项目定位

Sakichan SE 是 **opencode 编码 agent 的秘书前端**(原生 Android 应用)。

用户在手机上与"秘书" LLM 对话;秘书理解用户意图后,通过 opencode server HTTP API
在 PC 上派活、确认权限、总结结果。手机是遥控器,PC 是工作台,秘书是翻译官。

```
┌─────────────┐     自然语言      ┌──────────────┐    结构化指令    ┌──────────────────┐
│  用户(手机)  │ ──────────────► │  秘书 LLM     │ ──────────────► │  opencode server  │
│  Sakichan SE │ ◄────────────── │  DeepSeek V4F │ ◄────────────── │  (PC, headless)   │
└─────────────┘    流式回复+SSE   └──────────────┘   事件流+SSE     └──────────────────┘
```

**App 启动流程**:连接页 mDNS 扫描附近 opencode 机器 → 选机器连接(健康检查+拉项目
列表)→ 聊天页。多机器各管各的,session 组织成「机器 → 项目目录 → session」树。

## 2. 核心角色

| 角色 | 模型 / 引擎 | 端点 | 用途 |
|---|---|---|---|
| 秘书主脑 | DeepSeek V4 Flash (`deepseek-v4-flash`) | SenseNova token 端点 `https://token.sensenova.cn/v1/chat/completions` | 理解用户意图、function calling 派活、总结 opencode 输出。`reasoning_effort:"medium"`,`reasoning_content` 思考块需保留展示 |
| 图像理解 | SenseNova Flash-Lite (`sensenova-6.7-flash-lite`) | 同上 | (Phase 3)两阶段管线:先让 Flash-Lite 描述图片,再把描述喂给 DeepSeek(DeepSeek 不支持多模态) |
| 图像生成 | SenseNova U1-Fast | 桌面 config 有端点 | (Phase 3)秘书可调用生成图片 |
| 编码执行 | opencode (headless) | 局域网 `http://<PC-IP>:4096` | 实际编码 agent,接收指令、执行文件操作、返回 diff |
| 权限批准 | 用户(手机上) | — | opencode 请求改文件/跑命令时,手机弹 approve / deny / always |

## 3. opencode Server API(已按真实 schema 实现)

PC 端启动:`opencode serve --hostname 0.0.0.0 --port 4096 --mdns`

> ⚠️ 以下端点对照 **opencode 1.18.x 的 OpenAPI 3.1 spec**(`GET /doc` 抓取),
> 与本文档早期版本不同。已实现于 `OpencodeClient.kt`,改动 schema 时以此为准。

| 方法 | 路径 | 用途 |
|---|---|---|
| `POST` | `/session` | 创建 session,body `{parentID?, title?, agent?}`,query `directory?`(**必带**,避开失效项目) |
| `GET` | `/session` | 列出所有 session |
| `GET` | `/session/:id` | 获取 session 详情 |
| `GET` | `/session/:id/message` | **轮询消息列表——任务完成检测靠它(step-finish part)** |
| `POST` | `/session/:id/prompt_async` | **异步发消息**(204),配合事件流拿实时性 |
| `POST` | `/session/:id/permissions/:permissionID` | 权限回调,body `{response: "once"\|"always"\|"reject"}` |
| `POST` | `/session/:id/fork` | 分叉,body `{messageID?}` |
| `POST` | `/session/:id/abort` | 中止 |
| `GET` | **`/event?directory=<工作目录>`** | **真正的 per-directory SSE 事件流**(v1) |
| `GET` | `/api/event`、`/global/event` | 全局事件流(v2,payload 在 `data`/`payload` 字段) |
| `GET` | `/api/session/:id/permission` | 权限轮询(实测常空) |
| `GET` | `/global/health` | 探活 + 版本 |
| `GET` | `/project` / `/project/current` | 项目列表 / 当前项目(工作目录) |
| `GET` | `/file/content`、`/find` | 只读文件(Phase 3) |

> **Schema 实测修正(2026-08-04)**:`Project.vcs` 是**字符串**枚举(`"git"`),不是对象;
> `Project.time` 字段为 `created/updated/initialized`(旧文档 `modified/accessed` 已废弃)。
> 此前按旧文档把 `vcs` 声明成对象导致连接页 `listProjects` 抛 `unexpected json token`,
> 已改 `OpencodeModels.kt` 对齐真实 1.18.x spec。

### 3.1 事件流实测(与旧文档不符,以此为准)

BUILD.md 旧版写的是 `GET /api/session/:id/event` + `session.next.text.delta` 事件,
**实测 1.18.11 完全不推**(per-session 端点是空流)。真正可用的:

- **`GET /event?directory=<工作目录>`**(v1):目录级事件流,payload 在 `properties` 字段。
  核心事件:
  - `message.part.delta` → 流式文本增量,`properties.delta`(field=text/reasoning)
  - `message.part.updated` → part 快照,按 `properties.part.type` 分派
    (step-start / step-finish / text / reasoning / tool / error)
  - `session.idle` / `session.error` → 完成 / 出错
  - `session.status` → busy 等
- **完成信号**:事件流收不到 idle 时,app 用**轮询 `/session/:id/message`** 兜底,
  检测 assistant 消息含 `step-finish` part 即任务完成,180s 超时。
- **权限**:实测 `permission.asked` 事件不推、`/api/session/:id/permission` 轮询常空。
  手机上 approve/deny 仍走 `POST /session/:id/permissions/:permissionID`,
  但触发时机待 serve 行为稳定后再对齐。
- **建 session 必带 `directory` query**:serve 的项目注册表可能残留失效目录(如项目迁移前
  的旧路径),不带 directory 时新 session 落到死目录,agent 一跑就 ENOENT(SSE 500)。
   现 app 在连接时取 `/project/current` 或首个有效项目的 worktree 作为目录传入。

**任务执行链路(app 实测可用)**:`prompt_async` 发指令 → 订阅 `/event?directory=` 拿
`message.part.delta` 流式增量 → **轮询 `/session/:id/message` 兜底**,等 assistant 出现
`step-finish` part 即完成 → 拼输出回灌秘书总结。旧文档的 per-session SSE 端点
(`/api/session/:id/event`)与 `session.next.*` 事件在 1.18.11 实测不推送。

## 4. 秘书 ↔ App 协议:双层思考 agent(已重构)

早期设想的 `[#task]`/`[#done]` 文本标记**已废弃**。改用 function calling。秘书分两层:

**思考层**(`ThinkingAgent.kt`,DSV4F 硬编码、默认不开 reasoning_effort):
- 轻量自治 agent:思考 → 调只读 skill → 观察 → 再思考,循环直到终态
- 只读 skill(复用 opencode 只读 API):`list_directory` / `read_file` / `search_files`
- 终态工具:`ask_user`(要问用户)/ `start_task`(启动 opencode)/ `reply`(纯回复)
- 思考过程**不展示**给用户,只展示终态消息

**表层**(`ChatViewModel`):
- 用户可见的对话流,接收思考层终态
- opencode 运作时显示底部状态条(`OcStatus`,带动画)
- 用户在 oc 运行中可发消息问进度,思考层判断是汇报/继续等
- 秘书可主动发消息(oc 完成 / 需权限 / 主动汇报)

**对话流程**:
1. 用户下达初级想法 → 思考层理解 + 用 skill 收集上下文
2. 信息不足 → `ask_user` → 表层展示问题,等用户回复 → 回第 1 步
3. 已清楚 → `start_task` → 表层确认("好的,开始")→ 启动 opencode → 底部状态条
4. oc 运作中用户可问"怎么样了" → 思考层基于当前状态汇报
5. oc 完成 → 总结轮(`SUMMARY_PROMPT`,无工具)产出面向用户的总结

**思考层/总结轮均不带工具**(总结轮强制输出文本,防无限派活)。

## 5. 会话架构:本地对话线程 与 opencode 工作 session 解耦

用户确认的架构:**Sakichan 对话 session 和 opencode 工作 session 是两个独立的东西**。

- **Sakichan 本地会话**(`sakichanSessionId`,本地 uuid):秘书层的对话线程(用户↔秘书的
  聊天记忆),存在手机 DataStore。抽屉展示的是这些本地会话列表。
- **opencode session**(`opencodeSessionId`):PC 上真正干活的 agent 会话(独立 id)。
  **首次派活时才创建**(`ensureOpencodeSession` 懒创建),并绑定到当前本地会话。

**秘书的角色**:理解用户意图 → "选/建/切哪个 opencode session"**本身是工具指令** →
可主动创建、切换 session,或在不同 session 间跳。Sakichan 对话 session 只是秘书的记忆
容器,不是 opencode 工作的绑定。用户在 PC 上跑到一半,手机选中同一个 opencode session
即可续跑。

**抽屉 = 本地会话列表**(`refreshSessionTree` 从 `ChatHistoryRepository` 读),
不是 opencode 项目树。每个本地会话恢复其聊天 + 绑定的 opencode session id。

持久化模型 `PersistedChatSession`: `sessionId`(本地 uuid) + `opencodeSessionId`(绑定的
opencode session,可空)。

## 6. 技术栈

| 层 | 选型 | 说明 |
|---|---|---|
| UI | **Kanesumi**(Metro 风格) | 经 `includeBuild("../Kanesumi")` 组合构建接入。主题色注入 Sakichan 墨绿 `#1C5035` |
| 语言 | Kotlin 1.9.24 | 对齐 Kanesumi 工具链(AGP 8.5.0 / Compose BOM 2024.12.01 / Compose compiler 1.5.14) |
| 网络 | OkHttp 4.12 + okhttp-sse | SSE 解析(复用 legacy `ChatApiClient` + 新增 `OpencodeClient`) |
| DI | Koin 4.0.2 | `koin-compose` + `koin-compose-viewmodel`(非旧 `koin-androidx-compose`) |
| 序列化 | kotlinx-serialization **1.6.3** | ⚠️ 不能用 1.7.3:要求 Kotlin 2.0+,本项目锁 1.9.24 |
| 设置存储 | SharedPreferences + EncryptedSharedPreferences | API key 加密存储 |
| 持久化 | **DataStore Preferences + kotlinx JSON** | 聊天历史 + session 元数据缓存(`ChatHistoryRepository`) |
| 发现 | Android `NsdManager` | 扫描 `_http._tcp`,过滤 `opencode-*`(bonjour name) |
| compileSdk / minSdk | 36 / 26 | |

### 6.1 Kanesumi 组合构建

`settings.gradle.kts` 中:
```kotlin
includeBuild("../Kanesumi") {
    dependencySubstitution {
        substitute(module("io.github.takahashirinta:kanesumi-core")).using(project(":kanesumi-core"))
        substitute(module("io.github.takahashirinta:kanesumi-anim")).using(project(":kanesumi-anim"))
        substitute(module("io.github.takahashirinta:kanesumi-controls")).using(project(":kanesumi-controls"))
        substitute(module("io.github.takahashirinta:kanesumi-structure")).using(project(":kanesumi-structure"))
    }
}
```

Sakichan 与 Kanesumi 是兄弟目录(`/home/rain/projects/` 下),`../Kanesumi` 直接命中。
App 模块只需声明 `implementation("io.github.takahashirinta:kanesumi-structure")`,
structure 经 `api` 传递 controls/anim/core。Kanesumi 发布 Maven Central 后删 `includeBuild` 即可。

### 6.2 贡献回 Kanesumi 的组件(已完成)

Sakichan 实战自建、**已贡献回 Kanesumi** `kanesumi-controls`(commit f2e3f0b):

| 组件 | 用途 | 状态 |
|---|---|---|
| `MetroDrawer` | 汉堡抽屉(左侧滑出,session 列表) | ✅ 已在 Kanesumi |
| `MetroTextField` | 文本输入框(Metro 直角无圆角) | ✅ 已在 Kanesumi |
| `MetroChatInputBar` | 底部输入栏(挂 MetroBottomStack,自适应键盘) | ✅ 已在 Kanesumi,`sendIcon` 参数化(controls 零 M3 铁律) |

Sakichan 不再有本地副本,直接 import `io.github.takahashirinta.kanesumi.controls.*`。
**改这些组件的公共 API 时,注意另一个消费者 Ncrust。**

### 6.3 Metro 外观约束

- 零 M3:不依赖 `material3` / `material` / `material-icons-extended`
  (仅 `material-icons-core:1.7.5` 供 `ImageVector` 资产,无主题/组件)
- 直角切切:无圆角、无阴影、无边框(除非信息需要)
- 安全区:一律读 `MetroInsets`(`rememberMetroInsets()`),不用裸 `WindowInsets.*`
- 动画:用 `:kanesumi-anim` 的 Sokuou 预设,不散写 `tween(300, ...)`
- 默认即 Metro:`MetroTheme` 注入 `LocalIndication = MetroIndication`,所有 `.clickable {}` 免费获得直角闪切

### 6.4 本地持久化(Phase 2)

聊天历史与 session 元数据缓存,**DataStore Preferences + kotlinx JSON**,
而非 Room:数据量小、序列化与现有 kotlinx-serialization 栈一致,无需 KSP。

- `PersistedChat.kt`:可序列化模型。`PersistedChatSession`(sessionId/projectID/title/
  lastActiveAt/messages/items)+ `PersistedSessionMeta`(抽屉离线索引)+ `PersistedChatItem`
  (展示层 User/Secretary,任务与错误行是瞬时状态不缓存)
- `ChatHistoryRepository`:key `chat_<machineId>_<sessionId>` 存会话,**每机器一份索引**
  key `sessions_<machineId>`。`saveChat` / `updateIndex` / `loadChat` / `listSessions` /
  `deleteChat` 全部防御性解析(损坏即返回 null / 空)
- `ChatViewModel` 编排:
  - 每轮秘书回复 / 任务完成 / 总结轮后 `saveCurrentSession()`,断开连接时也存
  - `openSession()` 优先从本地缓存恢复历史(服务器不可达也回退缓存,不报错)
  - 重连后 `restoreLastSession()` 自动回到最近活跃的 session
  - `refreshSessionTree()` 成功后缓存元数据;服务器不可达时用缓存重建抽屉树
  - 抽屉每行加删除按钮,`deleteSessionCache()` 只清本地、不动服务器

## 7. 模块结构(当前实际)

```
Sakichan/
├── settings.gradle.kts          # includeBuild ../Kanesumi + dependencySubstitution
├── build.gradle.kts
├── gradle/
│   ├── libs.versions.toml       # 版本目录(kotlinx-serialization = 1.6.3)
│   └── wrapper/                 # gradle 9.3.1
├── app/
│   ├── build.gradle.kts
│   ├── proguard-rules.pro
│   └── src/main/
│       ├── AndroidManifest.xml  # usesCleartextTraffic=true + mDNS 权限
│       ├── kotlin/com/sakichan/se/
│       │   ├── SakichanApp.kt           # Koin 启动
│       │   ├── MainActivity.kt          # 连接状态驱动导航:CONNECTION ↔ CHAT ↔ SETTINGS
│       │   ├── connection/
│       │   │   └── ConnectionManager.kt # 活跃机器状态(StateFlow),驱动顶层导航
│       │   ├── core/
│       │   │   ├── error/SakichanError.kt
│       │   │   ├── model/
│       │   │   │   ├── Message.kt        # LLM 消息模型 + tool_calls
│       │   │   │   ├── Models.kt         # ChatOptions / FinalResult / Session
│       │   │   │   ├── Events.kt         # StreamEvent / PipelineEvent
│       │   │   │   ├── OpencodeModels.kt # opencode API 模型 + Machine + OcEvent
│       │   │   │   ├── PersistedChat.kt  # 本地持久化模型(聊天历史 + session 元数据)
│       │   │   │   └── SessionTree.kt    # 机器-项目-session 树 + buildSessionTree()
│       │   │   ├── session/SessionContext.kt
│       │   │   ├── agent/ThinkingAgent.kt # 思考层:自驱循环 + 只读 skill + 终态工具
│       │   │   └── util/                 # CjkUtils / Utils
│       │   ├── data/
│       │   │   ├── discovery/DiscoveryService.kt  # NsdManager 扫描
│       │   │   ├── network/ChatApiClient.kt       # LLM 流式 + tool_calls 归并
│       │   │   ├── network/OpencodeClient.kt      # opencode HTTP + SSE
│       │   │   └── repository/AppConfigRepository.kt + SecretaryPrompt.kt + ChatHistoryRepository.kt
│       │   ├── di/AppModule.kt           # Koin 模块
│       │   └── ui/
│       │       ├── theme/SakichanTheme.kt
│       │       ├── connection/ConnectionScreen.kt + ConnectionViewModel.kt
│       │       ├── chat/ChatScreen.kt + ChatViewModel.kt + ChatUiState.kt
│       │       └── settings/SettingsScreen.kt + SettingsHost.kt
│       └── res/
│           ├── drawable/ + mipmap-anydpi-v26/     # launcher 图标
│           └── values/{strings,colors,themes}.xml
├── assets/icons/                # 原始图标备份
└── README.md                    # 樱花标项目主页
```

## 8. 迁移历史(legacy 已删除)

旧桌面端(Rust GTK)+ 旧 mobile(Kotlin M3)曾移入 `legacy/` 作参考,**已于 2026-08-04
删除**(227MB,gitignored 从未跟踪,纯本地干扰 Android Studio)。旧 Rust checkout
`/home/rain/projects/Sakichan` 也已删除,其历史全在 git 中。

已迁移进新项目的有效物(改包名 `mobile` → `se`):`SakichanError` / `Events` /
`Message` / `Models` / `SessionContext` / `CjkUtils` / `Utils` / `ChatApiClient` /
`AppConfigRepository` / launcher 图标 / `colors.xml` / `themes.xml`。

## 9. 路线图(当前进度)

### Phase 1 — 最小闭环 ✅ 已完成
- ✅ `OpencodeClient`(session/prompt_async/SSE/权限/fork/health/projects)
- ✅ 秘书 prompt 工程 + function calling(`SecretaryPrompt.kt`)
- ✅ `ChatViewModel` 编排层(用户→秘书→opencode→总结→用户)
- ✅ `ChatScreen`(Metro 聊天 + 任务状态 + 权限条 + 思考块)
- ✅ `MetroTextField` / `MetroChatInputBar`(已贡献 Kanesumi)
- ✅ `SettingsScreen`(API key 加密 + 模型 ID)
- ✅ **启动连接页**(mDNS 扫描 + 手动输入 + 多机器)
- ✅ **session 树抽屉**(机器-项目-session)

### Phase 2 — 对话管理
- [x] **本地持久化** — DataStore Preferences + kotlinx JSON 缓存聊天历史 + session 元数据
- [ ] **deri fork UI** — 对话树可视化,基于 opencode session fork(客户端 API 已就绪)
- [ ] 自动生成 session 标题(opencode `/session/:id` PATCH title)

### Phase 3 — 增强
- [ ] 多模态 — 两阶段图片管线(Flash-Lite 分析 → DeepSeek 回答)
- [ ] 图像生成 — SenseNova U1-Fast
- [ ] 文件浏览 — opencode 只读 API(`/file/content`、`/find`)
- [ ] 权限通知 — opencode 权限请求推送通知

## 10. 架构决策(已拍板)

| # | 决策点 | 结论 |
|---|---|---|
| 1 | 编排放 PC bridge 还是手机直连 | **手机直连**。opencode server 本身就是 HTTP,无需额外 bridge(已确认不引入 Go 后端) |
| 2 | MVP 切多大 | 先跑通「发消息→执行→回传」闭环,已扩展出连接页 + session 树 |
| 3 | 标记生成机制 | **function calling**。`run_opencode_task` 工具,不用文本标记 |
| 4 | 组件归属 | Metro 组件**贡献回 Kanesumi**,不自成一套 |

## 11. 构建命令

```bash
./gradlew :app:assembleDebug        # debug APK -> app/build/outputs/apk/debug/
./gradlew :app:assembleRelease      # release APK(需签名配置)
./gradlew assemble                  # 全量构建
```

联调:`opencode serve --hostname 0.0.0.0 --port 4096 --mdns`

### 网络提示

本机访问 `services.gradle.org` 不稳定。首次构建若 gradle-9.3.1 分发下载超时,
用腾讯镜像手动放入:
```
https://mirrors.cloud.tencent.com/gradle/gradle-9.3.1-bin.zip
→ ~/.gradle/wrapper/dists/gradle-9.3.1-bin/<hash>/
```

## 12. 与 Ncrust / Kanesumi / Sokuou 的关系

- **Kanesumi** — UI 库,本仓库经 `includeBuild` 组合构建接入。改公共 API 时注意:
  Kanesumi 的另一个消费者 Ncrust 也会受影响。Kanesumi 发布 Maven Central 后,
  Sakichan 删掉 `includeBuild` 块即可切换远程依赖。
- **Sokuou** — Kanesumi 的动画引擎(`:kanesumi-anim` 移植自 Rust 桌面端 Sokuou)。
  Sakichan 不直接依赖 Sokuou,通过 Kanesumi 间接使用。
- **Ncrust** — 独立 app 项目(不在本仓库),也是 Kanesumi 的消费者。Sakichan 与 Ncrust
  无直接关系,但共享 Kanesumi 组件层。

## 13. 已知注意事项

- `gradlew` 已加执行权限;AGP 8.5.0 在 JDK 26 下有 Kotlin daemon 警告,可忽略
  (Kanesumi 用 JDK 21 工具链 `~/.gradle/jdks`)
- `legacy/` 已删:开发时无旧代码参考,需要时从 git 历史 `9fa31e1~1` 取
- mDNS 联调需 PC 带 `--mdns`;不带则手机手动输 IP
- 调试包 applicationId 带 `.debug` 后缀(`com.sakichan.se.debug`)
- **双 opencode 现象**:app 连接页出现多个"opencode"选项 = PC 上有多个 `opencode serve
  --mdns` 实例在广播。交互式 `opencode`(TUI)虽也起本地 server,但只绑 localhost,手机
  连不上;手机只能连 `serve --hostname 0.0.0.0 --mdns`。只想留一台就杀掉多余 serve。
- **失效项目目录**:`opencode serve` 启动后,项目注册表会记录当时 cwd;若目录后来被
  删除/迁移(如本仓库从 `~/Desktop/Sakichan` 迁到 `~/projects/Sakichan`),残留条目会让
  新建 session 落到死目录,agent 报 ENOENT。app 已通过建 session 时传 `directory` 规避;
  PC 侧彻底清理可删对应项目记录或重启 serve。
- **键盘**:输入栏/列表已加 `imePadding()`(edge-to-edge 下必须手动消费 IME insets,
  `adjustResize` 在 API 30+ 不自动生效),聊天列表底部留 `bottomOverlayPadding()` 防输入
  栏遮挡。
