# 插件系统（Plugins）实现方案

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[BACKLOG.md](./BACKLOG.md)（E1）、[LOOM_CLI.md](./LOOM_CLI.md)、[THEME.md](./THEME.md)、[CONTEXT_PANEL.md](./CONTEXT_PANEL.md)。

> **状态**：规格草案，**尚未实现**。须在核心能力（SSH、查找、主题等）相对稳定后，由用户**明确点名**再开工（与 backlog 纪律一致）。  
> **文档约定**：新增规格默认中文。

## 一句话目标

为 Loom 提供 **契约清晰、默认可沙箱、失败可熔断** 的插件机制：扩展主题、`loom` 子命令、snippets、状态栏等；插件加载或运行出错 **只废掉该插件**，不影响主程序；错误信息完整到足以定位（插件 id、阶段、源位置、栈）。

## 硬性约束

| 约束 | 含义 |
|------|------|
| 表达力强 | 能写控制流、数据、回调，并通过 Host API 调客户端能力——不只是改 JSON |
| 易上手 | 学习曲线短；优先大众脚本语言 + 清晰 API 文档 |
| 性能不差 | 冷路径（命令、batch、低频 UI）够用；**禁止**拖垮 PTY/每帧热路径 |
| 故障隔离 | 发现 / 加载 / 初始化 / 调用失败均不拖垮 Loom；可一键禁用全部插件 |
| 可诊断 | 结构化错误：phase、plugin_id、文件行列、栈、Host API 名；日志 + 设置页可复制 |

**默认否决**：无沙箱的同进程原生 `.dll` / `cdylib` 作为一等插件模型（一次 UB/未捕获 panic 即可拖垮进程，与隔离约束冲突）。原生高性能路径仅考虑后期 **WASM** 或 **sidecar 进程**。

## 与现有 backlog 对齐

E1 已定范围意向：

- **适合**：主题、状态栏碎片、snippets、自定义 profile 种类、`loom` 子命令（见 [LOOM_CLI.md](./LOOM_CLI.md)）。  
- **初期避免**：任意 **PTY 字节流** hook（延迟、卡 UI、破坏 VT、安全面）。

插件应调用稳定 **意图 API**（`pty.write`、`tabs.open`、`ui.toast`），而非钩进 GPUI Entity / russh / VT parser 内脏。

---

## 架构草图

```text
┌──────────────────────────────────────────┐
│ Loom Core（GPUI / PTY / SSH / UI）         │
│  PluginHost                               │
│   · registry / permissions / circuit break│
│   · Host API（稳定表面）                    │
│   · event bus（异步，不堵 PTY 读线程）       │
└───────────────┬──────────────────────────┘
                │
     ┌──────────┴──────────┐
     ▼                     ▼
  Lua VM（每插件独立）    WASM instance（可选二期）
     │                     │
     └── pcall / trap ─────┴──► PluginError → 日志 / 设置页 / toast
```

原则：

- 核心功能 **零依赖** 插件；先起主窗与会话，再 **延迟加载** 插件。  
- 插件结果全部按 `Result` 处理，Host **不对插件 `unwrap`**。  
- UI 更新必须回到 UI 线程队列；插件 worker 不得直接摸 GPUI。

---

## 可扩展点（分层）

### A. 低风险（MVP 优先）

| 扩展点 | 插件能力 | 失败时 |
|--------|----------|--------|
| **主题包** | UI tokens / 终端 palette（接 [THEME.md](./THEME.md)） | 回落内置主题 |
| **Snippets** | 注册或动态生成片段文本 | 不出现在列表 |
| **`loom` 子命令** | `loom foo` → Host API | 子命令缺失；其它 loom 仍可用 |
| **Batch 步骤类型** | 自定义一步（如通知） | 该 batch 步骤报错 |
| **命令面板条目** | 标题、关键字、action | 条目不注册 |
| **Status bar 片段** | 只读文本/图标、点击 | 占位「插件错误」 |
| **设置贡献** | 插件自身开关（插件配置目录） | 隐藏该段 |

### B. 中风险（第二期，需权限）

| 扩展点 | 说明 |
|--------|------|
| **Profile 种类** | 新连接后端描述与生命周期 |
| **Context 面板 Tab** | 自定义右栏页（优先声明式/受限组件，非任意 GPUI 树） |
| **传输钩子** | put/get 前后改路径或校验；需超时与失败策略 |
| **Host 执行扩展** | 扩大 `loom host` 可调程序（白名单） |
| **会话事件订阅** | Connected / Disconnected / CommandFinished（宜配 shell integration） |
| **键位贡献** | 仅新增；禁止默默覆盖核心绑定；需冲突检测 |

### C. 高风险（默认关闭或永不进程内开放）

| 扩展点 | 原因 |
|--------|------|
| 原始 PTY 读写 filter | 稳性/安全；与 E1 Avoid 一致 |
| 替换 VT / 渲染器 | 核心路径 |
| 任意 russh / SFTP 内脏 hook | 连接泄漏 |
| 每按键同步 hook | 卡输入 |
| 任意 GPUI 元素树 | panic 与生命周期难隔离 |

### 命令型 vs 事件型

| 类型 | 流向 | 要求 |
|------|------|------|
| **命令** | 用户 / CLI / 面板 → 插件 handler → Host API | 超时、权限、错误回传 |
| **事件** | Host 发出 → 插件可选订阅 | **异步**投递；插件慢不影响 PTY；可取消；错误只记该插件 |

---

## Manifest 与目录布局（草案）

```text
~/.loom/plugins/<plugin-id>/
  plugin.toml          # 或 plugin.json
  main.lua             # Lua 一等入口（MVP）
  # 或 plugin.wasm     # 二期
  themes/ …
  README.md
```

Manifest 示意：

```toml
id = "acme-deploy"
name = "Acme Deploy"
version = "1.2.0"
engines.loom = ">=0.1.0"

[contributes]
commands = ["acme.deploy"]
loom_subcommands = ["deploy"]
themes = ["themes/acme-dark.json"]
status_bar_items = ["acme.status"]

[permissions]
# 显式声明；未声明则调用对应 API → PermissionDenied
grant = ["ui.toast", "ui.commands", "pty.write"]
```

加载流程：发现 → 校验 semver/引擎/入口 → **独立 VM** 实例化 → `init` → 注册 contributes。任一步失败 → `PluginState::Failed`，**不注册**贡献项。

---

## 语言与运行时

### 选型结论（草案）

| 优先级 | 方案 | 角色 |
|--------|------|------|
| **一等（MVP）** | **Lua**（建议 `mlua`；可选 Luau） | 易上手、表达力够、可沙箱、嵌入成熟 |
| **二期** | **WASM**（wasmtime / Extism 等） | 更强隔离、多客语言（Rust/Go/AS）、性能好 |
| **可选加强** | **Sidecar 进程** + JSON-RPC | 不信任代码 / 原生扩展的隔离上限 |
| **不作为默认** | 同进程 Rust cdylib / 裸 DLL | 与「不影响整个 Loom」冲突 |
| **慎选默认** | 嵌入 CPython | 打包重、GIL、隔离差；若要 JS 则 QuickJS 或进程外 Deno，避免与 Lua 长期双养 |

「表达力」主要落在 **Host API 面**，语言只做胶水，便于将来换运行时。

### Host API 面（意图级示例）

```text
loom.tabs.open_profile(id)
loom.tabs.split(direction)
loom.pty.write(tab_id, text)
loom.cli.register_subcommand(name, handler)
loom.ui.toast(message)
loom.ui.command_palette.register(...)
loom.fs.read_under_root(path)      # 受限根
loom.sftp.get / put                # 需权限
loom.host.exec(argv)               # 高权限
loom.events.subscribe(kind, cb)    # 异步
```

禁止暴露：裸 `Window`/`Entity`、内部 VT parser、未包装的 russh session。

### 性能红线

| 允许 | 禁止 |
|------|------|
| 命令处理、batch 步骤、低频事件 | 每个 PTY byte 进入脚本 |
| 状态栏 500ms～1s 级刷新 | 每帧同步脚本布局 |
| Host 调度的后台任务 | 插件自建线程直接碰 UI |

单次 handler **软超时**（量级 50–200ms，可配置）；超时 → 取消、记 `PluginError`、可熔断该 handler。

---

## 故障隔离

### 加载期

1. 扫描插件目录；坏目录跳过并记错误。  
2. Manifest / 引擎版本失败 → Failed，不进入 init。  
3. **每插件独立 Lua state / WASM instance**（禁止共用可变全局污染）。  
4. 启动：**核心先就绪**，插件延迟或后台加载，避免「一个坏插件像打不开应用」。

### 运行期

| 机制 | 作用 |
|------|------|
| 入口 `pcall` / WASM trap | 脚本错不 unwind 穿出 Host |
| Host API 参数校验 | 非法 id/无权限 → `Result::Err`，不 assert |
| Worker + UI 队列 | 死循环不直接冻死渲染线程（配合超时） |
| **熔断** | 短期错误达 N 次 → 自动 Disable，需用户手动开 |
| **安全模式** | `loom --safe` 或设置「禁用全部插件」 |

### 进程级隔离（后期）

Sidecar：插件崩只杀子进程，Host 标 Failed并可重启该 sidecar。成本是 IPC 与打包；不阻塞 MVP。

---

## 错误模型（完整诊断）

### `PluginError` 字段

```text
plugin_id, plugin_version
phase: Discover | Manifest | Load | Init | Register | Call
capability          # 如 commands.execute / loom.cli.deploy
message             # 人话短句
detail              # 原始/VM 错误
source: { file, line, column }?
stack: [...]
host_api?           # 若在 Host 调用中失败
request_id?
timestamp
caused_by?
```

`phase` 必须有：区分「装坏了」与「点命令才炸」。

### 展示通道

| 通道 | 行为 |
|------|------|
| 日志文件 | 结构化（可 JSON 行），带 `plugin_id` |
| Settings → Plugins | 启用态 / 失败原因 / 打开日志 / 禁用 / **复制错误** |
| Toast | 短消息；详情进设置页 |
| （可选）`loom plugin doctor` | 打印全部 Failed 与栈 |

Manifest 解析错误须指出 **文件路径、键名、期望类型**。Lua 报错路径使用插件目录下真实路径，并附 `debug.traceback`。

### 用户路径（加载失败）

1. Loom 正常可用。  
2. 角标或设置提示「N plugins failed」。  
3. 详情示例：`acme-deploy@1.2.0 Init: module 'foo' not found @ …/main.lua:12`。  
4. 一键 Disable / 打开插件目录 / 复制完整错误。  

禁止：静默失败导致「命令消失且无解释」。

---

## 权限模型

首次启用插件时确认其 `permissions`（可记住）。

| Permission | 级别 | 含义 |
|------------|------|------|
| `ui.toast` / `ui.commands` | 低 | 提示与面板条目 |
| `pty.write` | 中 | 写入会话 |
| `fs.workspace` | 中 | 受限根文件 |
| `sftp.*` | 高 | 远端传输 |
| `host.exec` | 高 | 本机执行 |
| `net` | 高 | 网络 |

未授权调用 → `PermissionDenied`（完整 `PluginError`），不是崩溃。

---

## 分阶段交付

| 阶段 | 内容 | 验证点 |
|------|------|--------|
| **E1a** | 目录发现、Manifest、Failed 态、设置页列表、安全模式 | 坏插件不挡启动 |
| **E1b** | Lua 运行时 + 主题 / snippets / `loom` 子命令 / toast | 表达力与上手 |
| **E1c** | 权限、超时、熔断、结构化日志与复制错误 | 隔离 + 诊断 |
| **E1d** | 状态栏、命令面板、batch 步骤类型 | 中风险扩展 |
| **E1e** | WASM 与/或 sidecar | 更强隔离 / 多语言 |
| **明确后置或禁止** | PTY 字节 filter、任意原生 UI 树 | 稳性 |

建议依赖顺序：主题系统可切换（[THEME.md](./THEME.md)）与壳内 `loom` 网关（[LOOM_CLI.md](./LOOM_CLI.md)）有一定形状后，插件贡献这两类扩展点才自然。

---

## 验收意向（E1a～E1c）

- [ ] 故意破坏的插件（缺入口、语法错误、init 抛错）不阻止主窗与终端使用。  
- [ ] Failed 插件在设置中可见，且错误含 id、phase、路径/行列（若适用）。  
- [ ] 可一键禁用全部插件后行为与未装插件一致。  
- [ ] 合法插件可注册至少一种贡献（主题或 `loom` 子命令或 snippet）。  
- [ ] 无权限调用返回明确错误，不崩溃。  
- [ ] 超时或反复失败触发熔断，需手动恢复。

---

## 待决策（实现前写入 DECISIONS）

1. 一等语言是否 **仅 Lua**，WASM 是否同期。  
2. 分发形态：源码目录 vs 签名包装 vs 市场（市场可远超 E1）。  
3. `host.exec` / `net` 默认授予还是默认拒绝。  
4. Context 自定义 Tab：声明式 UI vs 脚本绘（隔离难度差一个数量级）。  
5. `engines.loom` 版本策略与破坏性 Host API 变更规则。

---

## 文档维护

- 本文是 **E1 规格草案**；拍板项进 [DECISIONS.md](./DECISIONS.md)。  
- 实现启动后更新文首状态，并将阶段表拆为具体任务。
