# BUILD.md — Sakichan SE 构建指南

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

## 2. 核心角色

| 角色 | 模型 / 引擎 | 端点 | 用途 |
|---|---|---|---|
| 秘书主脑 | DeepSeek V4 Flash (`deepseek-v4-flash`) | SenseNova token 端点 `https://token.sensenova.cn/v1/chat/completions` | 理解用户意图、生成结构化指令、总结 opencode 输出。`reasoning_effort:"medium"`,`reasoning_content` 思考块需保留展示 |
| 图像理解 | SenseNova Flash-Lite (`sensenova-6.7-flash-lite`) | 同上 | 两阶段管线:先让 Flash-Lite 描述图片,再把描述喂给 DeepSeek(DeepSeek 不支持多模态) |
| 图像生成 | SenseNova U1-Fast | 桌面 config 有端点,旧 mobile 未实现 | 秘书可调用生成图片(后续) |
| 编码执行 | opencode (headless) | 局域网 `http://<PC-IP>:4096` | 实际编码 agent,接收指令、执行文件操作、返回 diff |

## 3. opencode Server API

PC 端启动:`opencode serve --hostname 0.0.0.0 --port 4096 --mdns`

关键端点(OpenAPI 3.1):

| 方法 | 路径 | 用途 |
|---|---|---|
| `POST` | `/session` | 创建 session |
| `GET` | `/session` | 列出所有 session |
| `GET` | `/session/:id` | 获取 session 详情 |
| `POST` | `/session/:id/message` | 向 session 发消息(触发 agent 执行) |
| `GET` | `/session/:id/event` | SSE 事件流(token 输出、工具调用、权限请求等) |
| `POST` | `/session/:id/permissions/:id` | 权限确认回调(approve/deny) |
| `POST` | `/session/:id/fork?messageID=` | 从某消息分叉(deri 换思路) |
| `GET` | `/file/content` | 只读获取文件内容 |
| `GET` | `/find` | 只读文件搜索 |

事件流(`/event`)是核心:app 订阅 SSE,实时展示 agent 的 token 输出、工具调用进度、
权限请求(需用户在手机上 approve/deny)。

## 4. 标记语法(秘书 ↔ App 协议)

秘书 LLM 的回复中嵌入标记,App 解析后驱动 UI 状态机:

| 标记 | 形态 | 语义 |
|---|---|---|
| `[#task]` | 单行或 `[#task]...[/#task]` 围栏 | 开始一个 opencode 任务 |
| `[#confirm id]` | 单行 | 请求用户确认(对应 opencode 权限回调) |
| `[#reject id]` | 单行 | 用户拒绝(传回 opencode) |
| `[#done]` | 单行 | 任务完成 |
| `[#ask]` | 单行 | 秘书需要用户补充信息 |

**标记生成机制(待拍板)**:两个方向 --
1. **function calling**:让 DeepSeek 输出结构化 JSON,App 序列化为标记 -- 可靠但需 schema 设计
2. **prompt 围栏块**:在 system prompt 中教秘书产出标记文本 -- 灵活但可能不稳定

当前推荐方向 1(function calling),先跑通再优化。

## 5. deri — 对话树

旧 Sakichan 的 `(depth, order)` 对话树概念:用户可以从某条历史消息"换思路"分叉,
产生新的 order,同一 depth 下多个 order 形成树。

**新版映射**:opencode server 原生支持 `POST /session/:id/fork?messageID=`,
deri 树直接映射到 session fork。每次 fork 生成新 session,parentID/messageID 留痕,
UI 可以展示树形结构并切换分支。落地深度待定(MVP 可先只做线性,后续加 fork UI)。

## 6. 技术栈

| 层 | 选型 | 说明 |
|---|---|---|
| UI | **Kanesumi**(Metro 风格) | 经 `includeBuild("../Kanesumi")` 组合构建接入。主题色注入 Sakichan 墨绿 `#1C5035` |
| 语言 | Kotlin 1.9.24 | 对齐 Kanesumi 工具链(AGP 8.5.0 / Compose BOM 2024.12.01 / Compose compiler 1.5.14) |
| 网络 | OkHttp 4.12 + okhttp-sse | SSE 解析(复用 legacy `ChatApiClient`) |
| DI | Koin 4.0 | 轻量,无 ksp 依赖 |
| 序列化 | kotlinx-serialization 1.7.3 | LLM API JSON + opencode API JSON |
| 设置存储 | DataStore Preferences + EncryptedSharedPreferences | API key 加密存储,server URL 普通存储 |
| 持久化 | (待定)Room 或 DataStore | 聊天历史、session 缓存。旧 Room schema 需重构(适配 opencode session 模型) |
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

App 模块只需声明 `implementation("io.github.takahashirinta:kanesumi-structure")`,
structure 经 `api` 传递 controls/anim/core,下游一键触达全部组件。
Kanesumi 发布 Maven Central 后,删掉 `includeBuild` 块即可切换到远程依赖。

### 6.2 Kanesumi 缺失组件(需新建,有通用价值)

| 组件 | 用途 | 优先级 |
|---|---|---|
| `MetroDrawer` | 汉堡抽屉(左侧滑出,session 列表) | 高 |
| `MetroTextField` | 文本输入框(Metro 直角无圆角) | 高 |
| `MetroChatInputBar` | 底部输入栏(挂 MetroBottomStack,自适应键盘) | 高 |

这三个组件将在 Sakichan 实战中开发,完成后贡献回 Kanesumi 库。

### 6.3 Metro 外观约束

- 零 M3:不依赖 `material3` / `material` / `material-icons-extended`
  (仅 `material-icons-core:1.7.5` 供 `ImageVector` 资产,无主题/组件)
- 直角切切:无圆角、无阴影、无边框(除非信息需要)
- 安全区:一律读 `MetroInsets`(`rememberMetroInsets()`),不用裸 `WindowInsets.*`
- 动画:用 `:kanesumi-anim` 的 Sokuou 预设,不散写 `tween(300, ...)`
- 默认即 Metro:`MetroTheme` 注入 `LocalIndication = MetroIndication`,所有 `.clickable {}` 免费获得直角闪切

## 7. 模块结构

```
Sakichan/
├── settings.gradle.kts          # includeBuild Kanesumi + dependencySubstitution
├── build.gradle.kts
├── gradle/
│   ├── libs.versions.toml       # 版本目录(对齐 Kanesumi + app 专用)
│   └── wrapper/                 # gradle 9.3.1
├── app/
│   ├── build.gradle.kts
│   ├── proguard-rules.pro
│   └── src/main/
│       ├── AndroidManifest.xml  # usesCleartextTraffic=true(连局域网 HTTP)
│       ├── kotlin/com/sakichan/se/
│       │   ├── SakichanApp.kt           # Application(待接 Koin)
│       │   ├── MainActivity.kt          # Compose 入口,SakichanTheme + MetroShell
│       │   ├── core/
│       │   │   ├── error/SakichanError.kt
│       │   │   ├── model/Events.kt             # StreamEvent / PipelineEvent
│       │   │   ├── model/Message.kt            # LLM 消息模型 + JSON 序列化
│       │   │   ├── model/Models.kt             # ChatOptions / FinalResult / Session 等
│       │   │   ├── session/SessionContext.kt   # 滑动窗口截断 + deri 分支
│       │   │   └── util/
│       │   │       ├── CjkUtils.kt             # CJK 关键词提取(记忆检索)
│       │   │       └── Utils.kt                # TokenEstimator / Base64Utils
│       │   ├── data/
│       │   │   ├── network/ChatApiClient.kt    # OkHttp SSE 流式聊天
│       │   │   ├── network/OpencodeClient.kt   # (待建)opencode server API 客户端
│       │   │   └── repository/AppConfigRepository.kt  # API key + 模型配置
│       │   └── ui/
│       │       ├── theme/SakichanTheme.kt      # MetroTheme 封装,注入墨绿 primary
│       │       ├── chat/ChatScreen.kt          # (待建)聊天主界面
│       │       ├── chat/ChatViewModel.kt       # (待建)
│       │       └── settings/SettingsScreen.kt  # (待建)
│       └── res/
│           ├── drawable/                        # launcher 图标
│           ├── mipmap-anydpi-v26/               # adaptive icon
│           ├── values/{strings,colors,themes}.xml
│           └── values-zh/strings.xml
├── assets/icons/                # 原始图标备份
└── legacy/                      # (gitignored)旧桌面端 Rust + 旧 mobile,仅本地参考
```

## 8. 已迁移的有效物(legacy → 新项目)

| 文件 | 来源(legacy/mobile) | 改动 |
|---|---|---|
| `SakichanError.kt` | `core/error/SakichanError.kt` | 包名 `mobile` → `se` |
| `Events.kt` | `core/model/Events.kt` | 包名 |
| `Message.kt` | `core/model/Message.kt` | 包名 |
| `Models.kt` | `core/model/Models.kt` | 包名 |
| `SessionContext.kt` | `core/session/SessionContext.kt` | 包名 |
| `CjkUtils.kt` | `core/util/CjkUtils.kt` | 包名 |
| `Utils.kt` | `core/util/Utils.kt` | 包名 |
| `ChatApiClient.kt` | `data/network/ChatApiClient.kt` | 包名 |
| `AppConfigRepository.kt` | `data/repository/AppConfigRepository.kt` | 包名 |
| launcher 图标 | `res/drawable/` + `res/mipmap-anydpi-v26/` | 直接复制 |
| `colors.xml` | `res/values/colors.xml` | 直接复制(保留备用) |
| `themes.xml` | `res/values/themes.xml` | 简化:纯黑背景,去掉旧绿 |

### 未迁移(需重构)

| 文件 | 原因 |
|---|---|
| `ChatRepository.kt` | 架构变更:旧版直连 LLM,新版需加 opencode server 调度层 |
| `Repositories.kt` (SessionRepository/MemoryRepository) | 依赖 Room DAO,Room schema 需按 opencode session 模型重构 |
| `Daos.kt` / `MessageDao.kt` / `SessionDao.kt` | 同上 |
| `Entities.kt` | 同上 |
| `SakichanDatabase.kt` | 同上 |
| `Modules.kt` (Koin DI) | 需重写,适配新架构 |
| `ChatScreen.kt` / `ChatViewModel.kt` | M3 UI 全部废弃,用 Kanesumi Metro 重写 |
| `SettingsScreen.kt` | 同上 |
| `MarkdownParser.kt` / `MarkdownRenderer.kt` | 需评估是否保留(可能用 Kanesumi 组件替代) |
| `Color.kt` / `Theme.kt` / `Type.kt` | M3 主题全部废弃,用 `SakichanTheme`(MetroTheme 封装)替代 |

## 9. 待建路线图

### Phase 1 — 最小闭环(MVP)

目标:用户在手机上发消息,秘书调用 opencode server 执行,结果流式回传。

1. **`OpencodeClient`** — opencode server HTTP API 客户端
   - 创建 session、发消息、订阅 SSE 事件流
   - 权限回调(approve/deny)
   - JSON 模型对应 OpenAPI 3.1 schema
2. **秘书 prompt 工程** — system prompt 教秘书:
   - 理解用户编码意图
   - 产出 `[#task]` 标记 + opencode 指令
   - 监听 opencode 事件流,总结后回传用户
3. **`ChatViewModel`** — 编排层:
   - 用户消息 → 秘书 LLM
   - 秘书标记 → opencode server
   - opencode SSE → 秘书总结 → 用户
4. **`ChatScreen`** — Metro 聊天界面:
   - 消息列表(LazyColumn,MetroAppBar 作首项)
   - 底部输入栏(`MetroChatInputBar`,挂 `MetroBottomStack`)
   - 秘书思考块(`reasoning_content` 折叠展示)
   - 任务状态标记(`[#task]`/`[#done]` 视觉反馈)
5. **`MetroTextField` + `MetroChatInputBar`** — Kanesumi 新组件
6. **`SettingsScreen`** — 基础设置:
   - SenseNova API key(加密存储)
   - opencode server URL(如 `http://192.168.1.100:4096`)

### Phase 2 — 对话管理

7. **`MetroDrawer`** — 左侧汉堡抽屉,session 列表
8. **本地持久化** — Room 或 DataStore,缓存聊天历史 + session 元数据
9. **deri fork UI** — 对话树可视化,基于 opencode session fork

### Phase 3 — 增强

10. **多模态** — 两阶段图片管线(Flash-Lite 分析 → DeepSeek 回答)
11. **图像生成** — SenseNova U1-Fast 集成
12. **文件浏览** — opencode 只读 API(`/file/content`、`/find`)
13. **权限通知** — opencode 权限请求推送通知

## 10. 待拍板的架构决策

| # | 决策点 | 选项 | 当前倾向 |
|---|---|---|---|
| 1 | 编排放 PC bridge 还是手机直连 | A: 手机直连 opencode + 秘书 LLM<br>B: PC 上跑 bridge 服务,手机只连 bridge | A(手机直连):减少 PC 端组件,opencode server 本身就是 HTTP |
| 2 | MVP 切多大 | A: 仅秘书对话 + opencode 执行<br>B: 加 session 管理 + deri<br>C: 全功能 | A:先跑通"发消息→执行→回传"闭环 |
| 3 | 标记生成机制 | A: function calling(JSON)<br>B: prompt 围栏块(文本标记) | A:function calling 更可靠,先跑通 |

## 11. 构建命令

```bash
./gradlew :app:assembleDebug        # debug APK -> app/build/outputs/apk/debug/
./gradlew :app:assembleRelease      # release APK(需签名配置)
./gradlew assemble                  # 全量构建
```

### 网络提示

本机访问 `services.gradle.org` 不稳定。首次构建若 gradle-9.3.1 分发下载超时,
用腾讯镜像手动放入:
```
https://mirrors.cloud.tencent.com/gradle/gradle-9.3.1-bin.zip
→ ~/.gradle/wrapper/dists/gradle-9.3.1-bin/<hash>/
```
(Windows: `%USERPROFILE%\.gradle\wrapper\dists\gradle-9.3.1-bin\<hash>\`)

## 12. 与 Ncrust / Kanesumi / Sokuou 的关系

- **Kanesumi** — UI 库,本仓库经 `includeBuild` 组合构建接入。改公共 API 时注意:
  Kanesumi 的另一个消费者 Ncrust 也会受影响。Kanesumi 发布 Maven Central 后,
  Sakichan 删掉 `includeBuild` 块即可切换远程依赖。
- **Sokuou** — Kanesumi 的动画引擎(`:kanesumi-anim` 移植自 Rust 桌面端 Sokuou)。
  Sakichan 不直接依赖 Sokuou,通过 Kanesumi 间接使用。
- **Ncrust** — 独立 app 项目(不在本仓库),也是 Kanesumi 的消费者。Sakichan 与 Ncrust
  无直接关系,但共享 Kanesumi 组件层。

## 13. Git 历史

```
9fa31e1 chore: 旧桌面端与旧mobile移入legacy作参考,清空root准备秘书版重写
1c0f29e temp: 整合入原版移动端sakichan准备重塑
```

旧桌面端(Rust GTK)和旧 mobile(Kotlin M3)已移入 `legacy/`(gitignored),
仅本地参考。新项目从空白 root 重建,工具链对齐 Kanesumi,UI 全部 Kanesumi Metro。
