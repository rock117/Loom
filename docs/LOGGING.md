# 日志设计（对齐 Zed，分阶段落地）

相关文档：[HARD_PROBLEMS.md](./HARD_PROBLEMS.md)、[WINDOW_CLOSE_HANG.md](./WINDOW_CLOSE_HANG.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)。

> **状态**：设计说明（**尚未实现**）。  
> **日期**：2026-09-05。  
> **文档约定**：中文。

---

## 目标

1. 正式运行时可事后排障（尤其关窗卡住、SSH/SFTP）。
2. 默认不吵：不把 PTY 洪水、每帧 paint 打进文件。
3. 开发时有终端则打到 stderr；双击启动则打到文件。
4. 用环境变量按模块调级别，避免改代码重编。

---

## Zed 怎么做（对照）

| 点 | Zed |
| --- | --- |
| Sink | 有 TTY → stdout/stderr；否则 → `%LOCALAPPDATA%\Zed\logs\Zed.log`（macOS/Linux 有对应路径） |
| 回退 | 文件打不开 → stdout |
| 轮转 | 约 1 MiB 后 rename 为 old，再新建（简单，非按天多文件） |
| 级别 | 文件默认约 `info`；`ZED_LOG=info,project=debug` 细调（类 `RUST_LOG`） |
| API | 统一 `log` / 自研 `zlog`；业务不散落裸 `eprintln!` |
| 产品 | `zed: open log` / `reveal log in file manager`；文档写死路径 |

参考：Zed troubleshooting（Zed Log）、`crates/zlog`、启动时 `init_logger` / `zlog::init`。

---

## Loom 建议路径

| 用途 | 建议位置 |
| --- | --- |
| 配置 / workspace JSON | 已有：`%APPDATA%\Loom`（`platform::config_dir`） |
| **日志文件** | `%LOCALAPPDATA%\Loom\logs\Loom.log`（可重建，与 Zed 一样偏 Local） |
| 轮转备份 | 同目录 `Loom.log.old` |

macOS / Linux 实现时再定：`~/Library/Logs/Loom/` 或 `~/.local/share/loom/logs/`（与 Zed 习惯对齐即可）。

---

## 分阶段落地

### Phase 0 — 关窗诊断（最小，可先做）

仅服务 [WINDOW_CLOSE_HANG.md](./WINDOW_CLOSE_HANG.md)：

- 环境变量 `LOOM_QUIT_TRACE=1` 打开。
- 关窗路径 append 到 `Loom.log` 或 `eprintln!`，带时间戳与耗时。
- 阶段名固定，便于 grep：

```text
quit: should_close allow=false
quit: will_quit begin
quit: flush begin
quit: cwd pane=<id> pid=<n> begin
quit: cwd pane=<id> ok elapsed_ms=2
quit: persist ok
quit: cx.quit
```

若文件停在某次 `cwd … begin` 且无 `ok` → 坐实 UI 线程卡在 `process_cwd`。

**不必**先引入完整 log 框架。

### Phase 1 — Zed 同款骨架

1. 启动 `init_logger()`：TTY → stderr；否则 → `Loom.log`。
2. 依赖：`log` + 简单文件 writer（或日后 tracing / 精简 zlog）。
3. 超 ~1 MiB → rename `Loom.log.old` → 新建。
4. `LOOM_LOG` 或 `RUST_LOG` 控级别（默认 `info`）。
5. 关键路径用 `log::info!` / `log::warn!`；逐步收掉裸 `eprintln!`。

### Phase 2 — 产品体验

- 命令或设置：「打开日志」「在资源管理器中显示」。
- ARCHITECTURE / troubleshooting 写明路径。
- （可选）设置里「日志级别」——优先级低于环境变量。

---

## 原则

| 原则 | 做法 |
| --- | --- |
| 正式跑默认进文件 | 窗卡死时用户仍能打开 `.log` |
| 默认别吵 | 默认 `info`；禁止每 chunk PTY / 每帧 paint 打 debug |
| 可按模块开大 | 例：`LOOM_LOG=info,loom::ui::persistence=debug` |
| 关键路径要阶段名 | `quit.*`、`ssh.connect.*`、`sftp.transfer.*` |
| 日志勿堵 UI | 短消息；大 payload 截断；关窗诊断记 `elapsed_ms` |
| 失败可降级 | 文件失败 → stderr，应用照常启动 |

---

## 建议先覆盖的事件（Phase 1）

| 域 | 示例 |
| --- | --- |
| 生命周期 | 启动、`WillQuit`、flush、quit |
| 会话 | SSH connect 成功/失败/超时、disconnect |
| SFTP | 开 channel 失败、超预算、transfer 起止/取消 |
| 持久化 | `persist_now` 失败（已有 eprintln，可迁到 `log::error!`） |

不要：每次 keystroke、每次 forward `changes` notify、全量终端字节。

---

## 与现有代码的关系

- 今日：零星 `eprintln!`，无统一 logger，关窗路径无阶段日志。
- 配置目录：`src/platform.rs` / `src/platform/windows.rs`（`%APPDATA%\Loom`）。
- 关窗嫌疑：`docs/WINDOW_CLOSE_HANG.md` — Phase 0 专门验证。

---

## 非目标（本设计不做）

- 遥测上报 / 崩溃自动上传
- 按天保留 N 份复杂轮转
- 结构化 JSON 日志（除非日后排障证明需要）
- 在 UI 线程同步写超大文件

---

## 实施清单（未开始）

- [ ] Phase 0：`LOOM_QUIT_TRACE` + 关窗阶段日志
- [ ] Phase 1：`init_logger` + 文件路径 + 轮转 + `LOOM_LOG`
- [ ] Phase 2：打开/揭示日志 + 文档路径
- [ ] 用长会话 Bound Local 复现关窗，对照 `quit: cwd` 行验证 WINDOW_CLOSE_HANG
