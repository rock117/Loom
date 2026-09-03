# 平台 Shell 探测与后台子进程

相关文档：[LOCAL_SHELL.md](./LOCAL_SHELL.md)、[WINDOWS_SUBSYSTEM.md](./WINDOWS_SUBSYSTEM.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)。

> **状态**：Windows 已对齐 Zed 思路（路径扫描 + `LazyLock` 缓存 + `CREATE_NO_WINDOW`）；macOS / Linux 使用 Unix 常规 `$SHELL` 方案。  
> **文档约定**：规格与排障说明默认中文。

## 一句话结论

| 平台 | 默认 shell | 后台 `Command` |
|------|------------|----------------|
| **Windows** | 扫描常见安装路径 + `which` crate（**不** spawn `where.exe`），进程内缓存 | `platform::new_command` → `CREATE_NO_WINDOW` |
| **macOS** | 环境变量 `$SHELL`，fallback `/bin/zsh` | 普通 `Command::new`（无闪窗问题） |
| **Linux** | 环境变量 `$SHELL`，fallback `/bin/bash` | 同上 |

用户可在 **Settings → Default shell** 或 Profile 的 shell 字段写绝对路径或 PATH 名（如 `pwsh`），会 **优先于** 自动探测。

---

## 调用链

```text
TabManager::spawn_local
  → session::local::resolve_shell(configured)
    → platform::resolve_shell(configured)
      → 有配置且非空 → 原样返回
      → 否则 → platform::native_default_shell()
```

Local PTY spawn 使用解析后的路径/名称；集成终端里的 shell **应当** 有窗口（ConPTY / openpty），与后台 helper 不同。

---

## Windows（Zed 对齐）

### 背景

GUI 子系统下的 `loom.exe`（见 [WINDOWS_SUBSYSTEM.md](./WINDOWS_SUBSYSTEM.md)）没有控制台。若用裸 `std::process::Command` 启动 `where.exe`、`powershell` 等控制台程序，Windows 会 **短暂弹出黑色 CMD 窗口**。

早期 Loom 用 `where pwsh` 等子进程探测 PATH，会在 **启动或开 Local Tab** 时闪窗。现改为与 [Zed `gpui_util::get_powershell`](https://github.com/zed-industries/zed/blob/main/crates/gpui_util/src/lib.rs) 相同策略。

### 默认 shell 探测顺序

实现：`src/platform/windows.rs`，结果缓存在 `LazyLock`（**整个进程生命周期只算一次**）。

1. `Program Files\PowerShell\<版本>\pwsh.exe`（取最高数字版本；含 x86 备选目录）
2. Store / MSIX：`%LOCALAPPDATA%\Microsoft\WindowsApps\Microsoft.PowerShell_...\pwsh.exe`（含 Preview 包）
3. Scoop：`%USERPROFILE%\scoop\shims\pwsh.exe`
4. `~\.dotnet\tools\pwsh.exe`
5. `which::which_global("pwsh.exe")` / `powershell.exe`（Rust crate，Win32 PATH 查询，**不**起 `where.exe` 子进程）
6. `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`
7. 回退：`%SystemRoot%\System32\cmd.exe`

### 后台子进程

实现：`src/platform/command.rs`

```rust
platform::new_command("powershell")  // Windows: creation_flags(CREATE_NO_WINDOW)
```

**必须** 用于所有 **非终端 UI** 的后台 helper（Info 面板 GPU 探测、`nvidia-smi`、PowerShell CIM 等）。  
**不要** 用于：

- 集成终端里的 shell（ConPTY spawn，由 `portable-pty` 负责）
- `explorer` 打开文件夹（GUI 程序）

当前已使用：`src/session/host_info.rs` 的 `run_capture`。

---

## macOS

实现：`src/platform/macos.rs`

```rust
std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
```

- 登录 shell 通常已在 `$SHELL` 里（绝对路径，如 `/bin/zsh`、`/opt/homebrew/bin/fish`）。
- 无需 Windows 式路径扫描；也 **没有** `CREATE_NO_WINDOW` 等价需求。
- 打开 URL / 文件：`open`（GUI，不走 `new_command`）。

### 系统代理（Local Proxy Auto）

`src/session/local_proxy.rs` → `networksetup`（按 Wi-Fi / Ethernet / en0 等接口探测）。仍用裸 `Command::new`；Unix 上不会闪 CMD。

---

## Linux

实现：`src/platform/linux.rs`

```rust
std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
```

- 与 macOS 相同：信任 `$SHELL`，fallback `/bin/bash`。
- 打开 URL / 文件：`xdg-open`。
- 配置目录：`~/.config/loom`（小写，与 macOS/Windows 的 `Loom` 命名不同）。

### 系统代理（Local Proxy Auto）

`gsettings` 读 GNOME 代理（`org.gnome.system.proxy`）。KDE 等桌面环境可能需后续扩展。

### GPU 探测（Info 面板）

非 Windows：`lspci`（经 `run_capture` → `platform::new_command`，Unix 上与 `Command::new` 等价）。

---

## 与 Zed 的差异（有意保留）

| 项 | Zed | Loom |
|----|-----|------|
| Windows shell 扫描 | `gpui_util::get_powershell` | 同逻辑，在 `platform/windows.rs` |
| 后台 Command 封装 | `gpui_util::new_std_command` + `util::Command` | `platform::new_command` |
| Unix login shell | `$SHELL` / passwd（更完整时可读 getpwuid） | 仅 `$SHELL` + 固定 fallback |
| Settings | `terminal.shell.program` | `SettingsFile.default_shell` |

---

## 维护者检查清单

新增 **后台** 子进程时：

1. Windows 是否用了 `platform::new_command`？（避免闪 CMD）
2. 是否误把 `CREATE_NO_WINDOW` 加在 **终端 shell** 或 **explorer/open/xdg-open** 上？
3. 默认 shell 探测是否应避免每次 spawn 重复做 heavy 工作？（Windows 已 `LazyLock`；配置绝对路径仍是最稳）

---

## 相关路径

| 项 | 位置 |
|----|------|
| 统一 `resolve_shell` | `src/platform.rs` |
| Windows 探测 + 缓存 | `src/platform/windows.rs` |
| macOS / Linux 默认 shell | `src/platform/macos.rs`、`src/platform/linux.rs` |
| 后台 Command 封装 | `src/platform/command.rs` |
| Local PTY spawn | `src/session/local.rs` |
| GPU / host 探测 | `src/session/host_info.rs` |
| GUI 子系统 | `src/main.rs`、`docs/WINDOWS_SUBSYSTEM.md` |
