# Local shell（pwsh / cmd / 启动性能）

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[LOCAL_PROXY.md](./LOCAL_PROXY.md)、[SESSION_RECONNECT.md](./SESSION_RECONNECT.md)、[HARD_PROBLEMS.md](./HARD_PROBLEMS.md)。

> **状态**：Local PTY **已实现**；本文说明 shell 选型与「新开 Tab 慢」的常见原因与对策。  
> **范围**：仅 **Local** 会话；SSH 建连路径见 [SESSION_RECONNECT.md](./SESSION_RECONNECT.md)。  
> **文档约定**：规格与排障说明默认中文。

## 一句话结论

**Tab 打开慢，多数不是 GPUI 布局慢，而是所选 shell 进程冷启动慢。**  
在 Windows 上：**`cmd` 通常很快**，**`pwsh`（PowerShell 7+）往往明显更慢**；Loom 与其它终端一样，只是 spawn 你配置的 shell，无法 magically 跳过 PowerShell 自身的 profile 与初始化。

---

## pwsh 是什么？

| 可执行文件 | 常见名称 | 说明 |
|------------|----------|------|
| `pwsh.exe` | **PowerShell 7+** / PowerShell Core | 跨平台，微软主推的新版 |
| `powershell.exe` | **Windows PowerShell 5.1** | Windows 自带老版，仅 Windows |
| `cmd.exe` | **命令提示符** | 轻量，几乎无 profile |

日常说「装了 PowerShell 7」→ 命令行入口一般是 **`pwsh`**。  
Loom 在未配置默认 shell 时，Windows 上探测顺序为：**`pwsh` → `powershell` → `cmd`**（见 `src/platform/windows.rs`）。

---

## 其它终端支持 pwsh 吗？

**支持。** Tabby、Windows Terminal、WezTerm、Alacritty、Ghostty 等做法相同：

1. 创建 **PTY**（Windows 上多为 ConPTY）
2. 在 PTY 里 **spawn 用户配置的 shell**（cmd / pwsh / bash / …）
3. 把 stdin/stdout 接到终端 UI

它们**不实现 PowerShell**，只是启动进程。因此 **pwsh 在 Loom 里慢，在其它终端里新开 Tab 同样会慢**（除非用 `-NoProfile` 或精简 profile）。

---

## 慢在哪里？（分层）

```text
用户点击 Open / Ctrl+T
    → Loom UI（Tab、TerminalView）     ← 本地路径目前多为同步，会放大等待感
    → ConPTY / openpty                 ← Windows ConPTY 有固定成本
    → spawn shell（cmd / pwsh / …）    ← pwsh + profile 通常是最大头
    → Loom 注入的启动参数 / env        ← 见下文
    → 首屏 prompt / 首次字体渲染       ← 相对较小
```

### 1. Shell 自身（pwsh 慢、cmd 快的主因）

- **cmd**：无 `$PROFILE`，启动极快。
- **pwsh / powershell**：执行 **profile 脚本**、加载 **PSReadLine**、Oh My Posh、各类 `Import-Module`；企业环境还有策略与杀软扫描。
- 这是 **PowerShell 生态行为**，不是 Loom 独有。

### 2. Loom 为功能加的启动开销

Local spawn 时（`src/session/local.rs`）会：

- 设置 `TERM=xterm-256color`、`TERM_PROGRAM=Loom`
- 按 shell 类型注入 **OSC cwd 上报**（Reveal / Copy Path 等依赖 cwd）  
  - **cmd**：`PROMPT` 环境变量  
  - **pwsh / powershell**：`-NoExit -EncodedCommand` + 一段 hook（比 cmd 更重）
- 若 Settings → **Local Proxy** 为 **Auto**，spawn 前会探测系统代理（Windows 读注册表），见 [LOCAL_PROXY.md](./LOCAL_PROXY.md)

### 3. UI 路径差异（为何 SSH「感觉更快」）

| 路径 | Tab 何时出现 | 重活在哪 |
|------|----------------|----------|
| **SSH** | 立刻（`Connecting`） | 后台线程 `connect_blocking`，再挂 `TerminalView` |
| **Local** | PTY + 终端组件就绪后 | 当前多在 **UI 线程同步** `LocalPty::spawn` + `TerminalView::new` |

因此：**同样慢的网络 SSH 在「点下去」瞬间就有 Tab**；**本地 pwsh 要等 shell 起来才看到 Tab**，体感差距会被放大。详见 [HARD_PROBLEMS.md](./HARD_PROBLEMS.md) 中 UI 冻结检查项。

### 4. 默认 shell 探测（未配置时）

`platform::resolve_shell` 若 Settings / Profile 未写死路径，Windows 会对 `pwsh`、`powershell`、`cmd` 各跑一次 **`where`**（子进程）。建议 **在 Settings 里写绝对路径**，避免每次新建 Tab 都探测。

---

## 其它终端怎么处理「加载慢」？

业界没有统一「让 pwsh 变快」的魔法，常见是两类策略：

### A. 让等待不那么难受（产品层）

- Tab **先出现**，显示 Connecting / Starting…
- **后台** spawn shell，就绪后再 attach 终端（SSH 普遍如此；部分终端对 Local 也异步化）
- 文档说明：慢多半是 **PowerShell profile**，建议 `-NoProfile` 或精简 profile

### B. 不增加额外开销（实现层）

- 少注入启动脚本，或做成可选（shell integration）
- 缓存默认 shell 路径，避免每次 `where`
- 提供 Profile 启动参数字段（如 `-NoLogo`、`-NoProfile`）

Loom 当前：**SSH 已走 A**；**Local 仍以同步 spawn 为主**；**pwsh 会额外带 EncodedCommand cwd hook**。

---

## 用户侧：如何变快

### 1. 换默认 shell（最有效、已验证）

**Settings → Default shell** 设为：

```text
C:\Windows\System32\cmd.exe
```

或在 Local Profile 的 shell 字段写 **绝对路径**。  
需要 PowerShell 时再单独建 Profile 指向 pwsh。

### 2. 固定路径，关闭 Auto 探测

- Default shell 不要用空值依赖 `pwsh → powershell → cmd` 链式 `where`
- Local Proxy 若不需要：**Off**（见 [LOCAL_PROXY.md](./LOCAL_PROXY.md)）

### 3. 必须用 pwsh 时

- 精简 `$PROFILE`：延后 `Import-Module`、去掉重型主题
- 在 Profile 启动参数中加 **`-NoProfile`**（需自行在 Settings/Profile 配置；会跳过 profile，**部分 cwd / 交互增强可能变弱**）
- 对比：同一机器 Windows Terminal 新建 pwsh Tab，若同样慢 → 问题在 PowerShell，不在 Loom

### 4. 对比测试方法

1. Loom：同一 Profile 分别设 `cmd.exe` 与 `pwsh.exe`，各开 3 次 Tab，对比首屏时间  
2. Windows Terminal：同样 profile 命令行，对比冷启动  
3. 若仅 Loom 明显更慢且 cmd 也快 → 再查 Local Proxy Auto、同步 spawn UI 路径（提 issue / 见 BACKLOG）

---

## 代码入口（维护者）

| 模块 | 职责 |
|------|------|
| `src/platform/windows.rs` | 默认 shell 探测顺序、`where` |
| `src/platform.rs` | `resolve_shell` |
| `src/session/local.rs` | `LocalPty::spawn`、`configure_cwd_reporting`、teardown |
| `src/ui/tab_manager.rs` | `spawn_local`（同步）vs `begin_ssh` / `spawn_ssh_connect`（异步） |
| `src/session/local_proxy.rs` | Auto 时代理 env 注入 |

---

## 后续改进方向（未承诺排期）

- Local 与 SSH 对齐：**先 Tab + Connecting，后台 spawn PTY**（GPUI async / `background_spawn` 模式）
- Settings：**可选 `-NoProfile`**、可选关闭 cwd OSC hook
- 缓存 `resolve_shell` 结果，避免重复 `where`

若采纳架构变更，应在 [DECISIONS.md](./DECISIONS.md) 新增条目。

---

## 常见问题

**Q: pwsh 和 powershell 选哪个？**  
A: 需要新语法 / 跨平台用 **pwsh**；只要 Windows 老脚本可用 **powershell 5.1**；要快用 **cmd** 或 WSL bash。

**Q: 为什么 SSH 新建 Tab 不卡？**  
A: Tab 立即出现，连接在后台；不是 SSH 比 Local 轻，而是 **UX 异步**。

**Q: Loom 会比 Windows Terminal 更慢吗？**  
A: 同一 shell、同一 profile 下，**进程启动时间应接近**；Loom 可能多 cwd hook、代理探测、同步 UI 路径，差异通常小于 **pwsh profile 本身** 的开销。
