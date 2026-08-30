# Loom 壳内指令（`loom …`）与组合能力

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[BACKLOG.md](./BACKLOG.md)、[CONTEXT_PANEL.md](./CONTEXT_PANEL.md)、[THEME.md](./THEME.md)、[PORT_FORWARD.md](./PORT_FORWARD.md)、[DOCKER_SESSION.md](./DOCKER_SESSION.md)、[PLUGINS.md](./PLUGINS.md)。

> **状态**：需求挖掘 / 规格草案，**尚未实现**。实现须由用户明确点名后开始（与 backlog 纪律一致）。  
> **文档约定**：新增规格默认中文。  
> **来源**：产品讨论——要在 shell/SSH **提示符里直接输入并执行** 的客户端元命令（例如 `loom split` / `loom new`），并延伸批处理、宿主外部命令与指令组合。

## 一句话目标

在 Local / SSH 终端里，用户敲整行以 `loom` 开头的命令时，由 **Loom 客户端拦截并执行宿主能力**（分屏、开 tab、传文件、跑本机程序、批处理链等），**默认不把该行送进真 shell**；长流程可持久化为 batch，短流程支持 `&&` / `|` 等组合。

## 产品定位

| 是 | 不是 |
|----|------|
| 客户端元命令 / 轻量 DSL | 第二套远端 shell 语言 |
| 手不离终端的窗控与编排糖 | 替代命令面板（可并存，共用 action） |
| 本机↔远端的桥（SFTP、宿主进程、通知） | 伪装成 `ls`/`cd` 等日常 Unix 命令 |
| 可开关、可发现（`loom help`） | 默认劫持所有输入或不可关闭 |

与其它入口的关系：

| 入口 | 职责 |
|------|------|
| **壳内 `loom …`** | 焦点已在终端时的键盘路径（本文） |
| **命令面板 / 快捷键** | 鼠标与搜索；应调用同一批内部 action |
| **Snippets（P5）** | 把文本交给**真 shell**执行；不是元命令 |
| **Shell integration（OSC 133）** | 感知真命令起止/exit；强化 `wait-exit` / `rerun` / `notify`，不是本 DSL 前提 |
| **远端 PATH 里的真 `loom` CLI** | 备选诚实方案；本文主路径为 **输入拦截**，不要求每台 SSH 主机部署二进制 |

## 核心原则

1. **整行匹配**：trim 后匹配 `^loom(\s+.*)?$` 才拦截；`echo loom split` 进 shell。  
2. **前缀固定**：`loom`（建议大小写不敏感）；子命令用空格分隔。  
3. **客户端解析**：行内 `&&` / `|` 属于 **Loom 组合语言**，不会交给远端 bash；远端管道写作 `loom send "a | b"`。  
4. **种类分清**：UI / PTY(`send`) / Host(`run`) / Transfer / Data / Control 副作用不同，组合时要有类型意识。  
5. **可关**：Settings 如 `Intercept loom commands`；建议 Local 默认开，SSH 可单独开关或默认关。  
6. **危险要闸**：`host` 无白名单、`on`/`broadcast`/`parallel` 等需 confirm 或总开关。  
7. **命名反混淆**：宿主执行用 `loom host` / `loom run`（文案写明 **本机**），避免用户以为在服务器上跑。

## 实现要点（草案）

### 拦截模型（主路径）

1. `TerminalView` 维护**当前输入行缓冲**（可打印、Backspace、粘贴追加；尽量处理 Ctrl+U/C）。  
2. 收到提交（`\r`）：若缓冲为 loom 元命令 → **不** `write_to_pty`，解析并 dispatch；否则照常写入。  
3. Toast / 状态栏反馈成功或 `Unknown: loom foo`（未知子命令也不送 shell，避免误执行）。  
4. **Alt screen / 应用模式**（vim 等）下建议禁用拦截，减少误触。

### 改动区域（预估）

| 区域 | 内容 |
|------|------|
| `terminal/gpui_emu/view` | 行缓冲 + 提交拦截 |
| 新模块如 `loom_cli` | 词法/解析、子命令表、组合执行器 |
| `ui/workspace_view` 等 | 接收事件 → 复用现有 Split/NewTab/Files/Reconnect… |
| Settings / persist | 开关、别名、batch 路径 |
| （后期）shell integration | `wait-exit`、按命令切片 `out` |

难度粗估：MVP 仅 `help`/`new`/`split`/`close` ≈ **中低**；含 `send`/`host`/`put`/`get`+短链 ≈ **中**；完整 batch/管道/多机 ≈ **中高**。

---

## 积木类型

| 类型 | 例子 | 含义 |
|------|------|------|
| **UI** | `split` `new` `files` `sidebar` | 改客户端界面 |
| **PTY** | `send` | 往当前（或选定）会话写入，由真 shell 执行 |
| **Host** | `host` / `run` | 本机进程 |
| **Transfer** | `put` `get` `sync` | SFTP 或本地拷（接现有传输） |
| **Data** | `out` `cwd` `last` `set` | 产出文本/路径供下一环 |
| **Control** | `wait` `notify` `confirm` `sleep` | 节拍与门闩 |

---

## 指令目录（挖掘汇总）

下列为需求池，**非**承诺实现清单。标注：✓ 优先候选 · ～ 有价值后置 · ? 慎做/易踩坑。

### A. 窗格 / Tab

| 指令 | 说明 | |
|------|------|--|
| `loom new` | 新 tab（策略：固定 Local，或「克隆当前会话类型」需产品拍板） | ✓ |
| `loom new local` / `loom new ssh` | 指定类型 | ✓ |
| `loom dup` | 复制当前 tab | ✓ |
| `loom split` / `loom split left\|right\|up\|down` | 分屏（默认 right，对齐 Ctrl+\） | ✓ |
| `loom close` / `loom close tab` | 关 pane / 关 tab | ✓ |
| `loom only` | 只留当前 pane | ～ |
| `loom tab 3` / `next` / `prev` | 切 tab | ✓ |
| `loom tabs` | 列出 tab | ～ |
| `loom zoom` | 专注/放大当前 pane | ～ |

### B. 布局 / 面板 / 外观

| 指令 | 说明 | |
|------|------|--|
| `loom sidebar` / `loom panel` | 切换侧栏 / 右栏 | ✓ |
| `loom files` / `loom info` | 打开并聚焦 Context 页 | ✓ |
| `loom settings` | 设置 | ✓ |
| `loom font +` / `-` / reset | 字号 | ✓ |
| `loom numbers` | 行号开关 | ✓ |
| `loom theme dark\|light` | 依赖主题系统 [THEME.md](./THEME.md) | ～ |
| `loom layout save\|load <name>` | 分屏树持久化 | ～ |
| `loom always-on-top` / `loom screenshot` | 窗口级糖 | ～ |

### C. 连接 / Profile

| 指令 | 说明 | |
|------|------|--|
| `loom reconnect` | SSH 重连；Local 提示 N/A | ✓ |
| `loom open <name>` | 模糊打开 profile | ✓ |
| `loom ssh <host>` | 预填新建或临时连 | ～ |
| `loom disconnect` | 断连留 tab | ? |
| `loom boot` | 跑 profile 启动命令（P3） | ～ |
| `loom forward` / `loom port` | 端口转发 UI（[PORT_FORWARD.md](./PORT_FORWARD.md)） | ～ |
| `loom docker` | 容器会话（[DOCKER_SESSION.md](./DOCKER_SESSION.md)） | ～ |
| `loom template <name>` | 会话套装（P2） | ～ |

### D. 路径 / 文件 / 传输

| 指令 | 说明 | |
|------|------|--|
| `loom cwd` / `loom pwd` / `loom copy-cwd` | 显示或复制 cwd | ✓ |
| `loom cd`（无参） | Files 跳到终端 cwd | ✓ |
| `loom reveal` / `loom explorer` | 本机打开 cwd（SSH 策略另定） | ✓ |
| `loom get <remote>` / `loom put <local>` | 下载/上传（支持通配、`-r`） | ✓ |
| `loom sync <local> <remote>` | 单向同步 | ～ |
| `loom edit <file>` | get → 本机编辑器 → put 回 | ～ |
| `loom diff local remote` | 本机 vs 远端 | ～ |
| `loom mkdir`（Files） | 与 shell 重复，低优先级 | ? |
| `loom queue` | 传输队列指令化（接 Transfers 页脚） | ～ |
| `loom mount` | SFTP 缓存式「映射」观感，非真网盘 | ? |

### E. PTY 与片段

| 指令 | 说明 | |
|------|------|--|
| `loom send <cmd>` | 写入 PTY（批处理积木） | ✓ |
| `loom snip` / `loom snip <name>` | 列表或注入片段（P5） | ✓ |
| `loom snip add` | 上一条真命令存片段（宜配 integration） | ～ |
| `loom rerun` / `loom last` | 重跑/显示上一条真命令 | ～ |
| `loom clear` | 清 scrollback（或勿与 shell `clear` 抢名） | ～ |
| `loom find <text>` | 打开查找并填词 | ✓ |
| `loom copy` / `select-all` | 选区/缓冲 | ～ |

### F. 宿主外部命令（本机）

| 指令 | 说明 | |
|------|------|--|
| `loom host <prog> [args…]` / `loom run …` | 本机执行；cwd 可跟 OSC cwd | ✓ |
| `loom open-url <url>` | 系统打开 URL | ～ |
| `loom pipe <prog>` | 选区或最近输出送到本机进程 stdin | ～ |
| 白名单目录脚本 | 如 `~/.loom/scripts/` | ～ |

### G. 输出与数据

| 指令 | 说明 | |
|------|------|--|
| `loom out` / `loom save-out` / `loom copy-out` | 导出或复制缓冲 / 上一段命令输出 | ✓ |
| `loom copy-last` | 上一条命令文本 | ～ |
| `loom slice N:M` | 按行导出 | ～ |
| `loom mark` / `loom jump` | scrollback 书签 | ～ |
| `loom pick` | 从输出交互选行/路径（迷你 fzf） | ～ |
| `loom table` / `loom json` | 弱结构化解析 | ? |
| `loom snapshot` / `loom diff`（两次输出） | 小众 | ? |

### H. 作业 / 通知 / 调度

| 指令 | 说明 | |
|------|------|--|
| `loom notify` / `loom notify on` | 下一条真命令结束通知（T4） | ✓ |
| `loom wait` / `loom wait-exit` / `loom sleep N` | 批处理同步（强依赖 integration 才稳） | ✓ |
| `loom watch-exit` | tab 着色/铃 | ～ |
| `loom timing` | 下一条真命令计时 | ～ |
| `loom job start\|status\|kill` | 宿主长任务可观察 | ～ |
| `loom schedule` / `loom idle-run` | 需托盘/常驻（S1），慎 | ? |
| `loom silence` | 暂时吞 toast | ～ |

### I. 多会话编排

| 指令 | 说明 | |
|------|------|--|
| `loom on a,b -- send "…"` | 多 tab/profile 发送 | ～ |
| `loom broadcast "…"` | 当前窗所有 pane | ～ |
| `loom join` / `loom group` / `loom fan` | 编组与汇总 exit | ～ |
| `loom focus matching <pat>` | 按标题聚焦 | ～ |
| `loom parallel …` | 并行；默认应关、需确认 | ? |

### J. 安全 / 变量 / 上下文

| 指令 | 说明 | |
|------|------|--|
| `loom env` / `loom ctx` | 变量与会话摘要 | ✓ |
| `loom set` / `loom unset` | 会话变量，供 `{{name}}` | ✓ |
| `loom pin cwd` | 钉住 Files/传输根 | ～ |
| `loom secret set\|get` | 钥匙串；展开时注意 scrollback 泄漏 | ～ |
| `loom redact on` | 展示层打码 | ? |
| `loom audit` | 本地批处理审计日志 | ～ |
| `loom safe` | 禁用 host/on/parallel 的安全模式 | ～ |
| `loom confirm` | 批处理人工闸 | ✓ |
| `loom dry-run <batch\|chain>` | 只打印步骤 | ✓ |

### K. 帮助 / 发现 / 人机

| 指令 | 说明 | |
|------|------|--|
| `loom` / `loom help` / `loom help split` | 发现 | ✓ |
| `loom version` | 版本 | ✓ |
| `loom alias` | 短别名 | ～ |
| `loom !!` | 重跑上一条 **loom** 链 | ～ |
| `loom palette` | 拉起命令面板 | ～ |
| `loom choose profile\|snip` | 模糊选择 | ～ |
| 多行块 | `loom { … }` 或 heredoc 式 | ～ |

### L. 录制与自定义命令

| 指令 | 说明 | |
|------|------|--|
| `loom rec start\|stop [name]` | 录成 batch | ～ |
| `loom play <file>` | 回放（可停在 confirm） | ～ |
| `loom batch <name>` | 跑命名批处理 | ✓ |
| 用户命令 | `~/.loom/commands/*.loom` → `loom foo` | ～ |
| 插件 | `loom plugin`（E1 后） | ～ |

### M. 明确不建议 / 非目标

| 项 | 原因 |
|----|------|
| `loom ls` / `loom cd` 替代真 shell | 语义冲突、破坏终端诚实性 |
| 拦截非 `loom` 前缀的任意输入 | 过大、难预测 |
| 任意 PTY 字节流插件 hook | 安全与稳定风险（见 backlog E1） |
| 社交录像分享、跨用户协同会话 | 与现 icebox 排除项一致 |
| 无确认的本机任意 `host` + 多机 `parallel` | 杀伤面过大 |
| 把自然语言 `loom ask` 绑成核心 | 可后置实验，勿阻塞 DSL |

---

## 指令组合

### 组合模型

```
单指令     loom split
短链       loom put … && loom send … && loom notify
管道       loom get f | loom host bat
作用域     loom on a,b -- send "…"
长组合     loom batch deploy  /  loom rec
```

### 建议语法

| 风格 | 示例 | 用途 |
|------|------|------|
| 仿 shell 短链 | `loom put .\x && loom send "restart" && loom notify` | 临时组合；`&&` 失败停，`;` 继续 |
| 管道（收窄） | `loom out \| loom host rg error` | 仅 **Data → Host/clip/save** |
| 持久 batch | 一行一步的文件或 JSON | 长流程 |
| 显式 `do`（备选） | `loom do split -- files` | 若仿 shell 解析痛苦再用 |

占位符（草案）：`{{cwd}}` `{{profile}}` `{{host}}` `{{user}}` `{{tab}}` `{{local}}` `{{remote}}` 以及 `loom set` 的自定义键。

### 额外算子（后置挖掘）

| 算子 | 含义 |
|------|------|
| `\|\|` | 上一步失败才执行 |
| `each` / 对路径列表映射 | 批量 get/host |
| `@file` | 参数列表从文件读 |
| `try` / `catch` / `retry N` / `timeout` | batch 容错 |
| `with profile X { … }` | 临时作用域块 |

### 组合戒律

1. 组合在**客户端**解析，不等于远端 shell。  
2. 管道默认只打通 data→host，避免无意义的 UI|UI。  
3. 含 `on` / `parallel` / 无白名单 `host` 必须可闸。  
4. UI 步骤默认成功；Transfer/Host 使用真实退出码。

### 示例故事

1. **发布**：`loom put .\publish\** /opt/app && loom send "systemctl restart app" && loom notify`  
2. **日志回本机**：`loom get /var/log/app.log | loom host bat`  
3. **开工作区**：`loom open api && loom split && loom open db && loom files`  
4. **录制复用**：操作后 `loom rec save publish` → 日常 `loom batch publish`

### Batch 文件（方向）

- 短链不够时落盘；格式待定（一行一命令 DSL vs JSON 步骤数组）。  
- 步骤类型对齐积木表；支持 `dry-run` / `confirm`。  
- 用户自定义 `loom <name>` 可指向 batch 别名。

---

## 分阶段建议（实现时再拆任务）

| 阶段 | 内容 | 难度 |
|------|------|------|
| **0** | 拦截框架 + 开关 + `help` / `version` | 低 |
| **1** | 窗控：`new` `dup` `split*` `close` `tab` `sidebar` `files` `info` | 中低 |
| **2** | 会话糖：`reconnect` `open` `cwd` `reveal` `find` `settings` `font` | 中低 |
| **3** | 积木：`send` `host` `put`/`get` + `&&` 短链 + `notify`/`confirm`/`dry-run` | 中 |
| **4** | Data 管道：`out`/`save-out` `|` host；变量 `set`/`env` | 中 |
| **5** | `batch` + `rec`；用户命令目录 | 中高 |
| **6** | `on`/`broadcast`；`wait-exit`（integration）；`edit`/`pick` | 中高 |
| **7** | 插件子命令、调度类 | 高 / 远景 |

命令面板可与阶段 1 并行：暴露同一 action，不替代壳内路径。

---

## 验收意向（阶段 1～2）

- [ ] 在 Local 终端输入 `loom help` / `loom split` 不进入 shell，并产生对应 UI 效果。  
- [ ] 非整行前缀（如 `echo loom split`）仍进 shell。  
- [ ] 设置可关闭拦截。  
- [ ] 未知子命令有反馈且不送 PTY。  
- [ ] （阶段 3）`loom put … && loom send …` 按客户端链执行，远端看不到 `&&` 字面元命令行。

---

## 与 backlog 条目的映射

| Backlog | 关系 |
|---------|------|
| P3 Profile startup | `loom boot` |
| P2 Templates | `loom template` / 组合开多 tab |
| P5 Snippets | `loom snip*`；与 `send` 分工 |
| P1c / P1b | `forward` / `docker` 入口 |
| T4 Notifications | `loom notify` |
| A1 Theme | `loom theme` |
| S1 Tray | `schedule` 等才有载体 |
| E1 Plugins | 自定义 `loom` 子命令挂载 |

---

## 文档维护

- 本文件是 **需求与方向汇总**；拍板后的硬性决策（语法终稿、SSH 默认开关、`new` 克隆策略等）应另记 [DECISIONS.md](./DECISIONS.md)。  
- 实现启动后，可将「阶段表」拆成具体 PR 任务，并在此更新状态行。
