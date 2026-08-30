# Windows 子系统与纯 GUI 启动

知识笔记：说明 `#![windows_subsystem = "windows"]` 与 Loom 在 Windows 上不弹 CMD 的原因。不属于架构规格；需要时也可对照 [HARD_PROBLEMS.md](./HARD_PROBLEMS.md) 中的平台坑。

## 结论（Loom 现状）

`src/main.rs` 顶部有：

```rust
#![windows_subsystem = "windows"]
```

因此 Windows 上的 `loom.exe` 按 **GUI 子系统** 链接；双击或从资源管理器启动时 **不会** 自动弹出黑色 CMD 窗口。

---

## `#![windows_subsystem = "windows"]` 是什么

这是 Rust 的 **crate 级属性**（`#!` 作用于整个 crate），只在 **Windows 目标**上有意义。

它告诉链接器：把可执行文件的 PE 头标成：

| 属性值 | PE Subsystem | 含义 |
|--------|--------------|------|
| （默认，不写） | `IMAGE_SUBSYSTEM_WINDOWS_CUI` | **Console** 控制台程序 |
| `"windows"` | `IMAGE_SUBSYSTEM_WINDOWS_GUI` | **Windows** GUI 程序 |
| `"console"` | 同上 CUI | 显式指定控制台 |

Windows 创建进程时会读这个字段，决定是否为进程 **分配控制台**。

在 macOS / Linux 上该属性基本被忽略，可保留在跨平台源码里。

---

## Console vs Windows：启动行为

| | Console（默认） | Windows（纯 GUI） |
|---|---|---|
| 双击 / 资源管理器启动 | **先开一个控制台窗口**（黑 CMD） | **不分配控制台** |
| 已有终端里运行 `.\loom.exe` | 沿用当前控制台；`println!` 可见 | 通常 **不** 挂接控制台；标准输出常“看不见” |
| 典型用途 | CLI、带日志的工具 | 桌面 GUI |

“运行 loom.exe 会弹 CMD” 的根因，就是子系统仍是 Console，而不是 GPUI 自己画了一个终端窗。

---

## 它不负责的事

- **不是** 隐藏 / 最小化应用主窗口，也不改 GPUI 窗口行为  
- **不是** 禁止程序自己再 `AllocConsole` 或重定向日志  
- **不替代** 图标、manifest、DPI 感知等（Loom 图标仍由 `build.rs` + `resources/windows/loom.rc` 嵌入）  
- **不等于** 子进程不闪黑窗（见下文）

业务入口仍是 Rust 的 `fn main()`；不必改成 Win32 的 `WinMain`。工具链会按子系统接好启动入口。

---

## 对调试与日志的影响

纯 GUI 后常见现象：

- `cargo run` 时终端里可能看不到 `println!` / `eprintln!`  
- panic 默认写 stderr，也可能像“闪退、无输出”

若希望 **Debug 仍带控制台、Release 才纯 GUI**，可用：

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
```

Loom 当前采用 **无条件** `"windows"`：Debug / Release 启动都不弹 CMD。需要看日志时，用调试器、文件日志，或临时改回 console / 使用上面的 `cfg_attr` 折中。

---

## 和「子进程弹 CMD」的区别

| 机制 | 管谁 | Loom 里的例子 |
|------|------|----------------|
| `windows_subsystem` | **本进程** `loom.exe` 启动时要不要控制台 | `src/main.rs` |
| `CREATE_NO_WINDOW`（`Command` creation flags） | **子进程** 会不会闪黑窗 | 如 Info 面板采 GPU 时跑 `nvidia-smi` / PowerShell |

两者独立。父进程已是 GUI，子进程若以默认 console 方式 `Command::new` 启动，仍可能短暂弹出控制台；需要时再对子进程设 `CREATE_NO_WINDOW`（或等效重定向）。

---

## 如何自查

在已构建的 `loom.exe` 上可用（任选）：

- Visual Studio：`dumpbin /headers loom.exe` → 找 `subsystem`
- 第三方 / 脚本读 PE Optional Header 的 Subsystem 字段  
  - `3` = Windows CUI（console）  
  - `2` = Windows GUI  

改属性后必须 **重新链接**（完整 `cargo build`）才会写进新的 PE；只改源码不重建无效。

---

## 相关路径

| 项 | 位置 |
|----|------|
| 子系统属性 | `src/main.rs` |
| Windows 图标资源 | `build.rs`、`resources/windows/loom.rc`、`assets/icons/loom.ico` |
| 平台封装 | `src/platform/`（与子系统无关，但同属 Windows 打包面） |
