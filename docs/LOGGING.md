# 日志设计（对齐 Zed，分阶段落地）

相关文档：[HARD_PROBLEMS.md](./HARD_PROBLEMS.md)、[WINDOW_CLOSE_HANG.md](./WINDOW_CLOSE_HANG.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)。

> **状态**：已实现 Phase 0–2（`init_logger`、关窗阶段日志、Settings → Logging）。  
> **日期**：2026-09-05（2026-09-17：补充 Settings；落地实现）。  
> **文档约定**：中文。

---

## 目标

1. 正式运行时可事后排障（尤其关窗卡住、SSH/SFTP）。
2. 默认不吵：不把 PTY 洪水、每帧 paint 打进文件。
3. 开发时有终端则打到 stderr；双击启动则打到文件。
4. **Settings 里可改级别 / 存储位置**；环境变量可临时覆盖（不必改代码重编）。

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

## Loom 建议路径（默认）

| 用途 | 建议位置 |
| --- | --- |
| 配置 / workspace JSON | 已有：`%APPDATA%\Loom`（`platform::config_dir`） |
| **日志文件（默认）** | `%LOCALAPPDATA%\Loom\logs\Loom.log`（可重建，与 Zed 一样偏 Local） |
| 轮转备份 | 同目录 `Loom.log.old` |

用户可在 **Settings → Logging** 改目录；未改时用上表默认。  
macOS / Linux 默认：`~/Library/Logs/Loom/` 或 `~/.local/share/loom/logs/`。

---

## Settings 配置（产品面）

设置面板增加 **Logging** 分区（与 Shell / Font / Proxy 同级），持久化进 `settings.json`（`SettingsFile`）。

### 字段

| Settings 项 | 建议字段 | 默认 | 说明 |
| --- | --- | --- | --- |
| 启用文件日志 | `logging.enabled` | `true`（无 TTY / 正式启动时） | 关闭则尽量只打 stderr（有 TTY）或丢弃 |
| 级别 | `logging.level` | `info` | `error` / `warn` / `info` / `debug` / `trace` |
| 日志目录 | `logging.dir` | 空 = 平台默认 | 目录；文件名固定 `Loom.log`（避免用户写错扩展名） |
| （可选）最大文件 | `logging.max_bytes` | `1 MiB` | 超过则轮转为 `Loom.log.old` |

### 操作按钮（同区）

- **打开日志**：用系统默认程序打开当前 `Loom.log`（不存在则创建空文件或提示）。
- **在资源管理器中显示**：`reveal` 日志目录 / 文件。
- **打开日志目录**：同上，偏文件夹。

改级别 / 目录后：尽量 **热更新** logger（不必重启）；若实现困难，Settings 文案写「部分变更需重启」。

### 优先级（高 → 低）

```text
1. 环境变量 LOOM_LOG / LOOM_LOG_DIR / LOOM_QUIT_TRACE   （临时排障、CI）
2. Settings（settings.json）
3. 内置默认（info + 平台默认目录）
```

例：Settings 为 `warn`，启动前设 `LOOM_LOG=debug` → 本次进程用 `debug`。

### UI 示意

```text
Logging
  [x] Write log file
  Level          [ info ▾ ]
  Log folder     [ %LOCALAPPDATA%\Loom\logs ]  [Browse…]
  [ Open log ]  [ Reveal in Explorer ]
```

路径字段用现有 `RenameEdit` + Browse（与 Local start directory 同类）；勿用裸 `String` 无选区。

---

## 分阶段落地

### Phase 0 — 关窗诊断（最小，可先做）

仅服务 [WINDOW_CLOSE_HANG.md](./WINDOW_CLOSE_HANG.md)：

- 环境变量 `LOOM_QUIT_TRACE=1` 打开。
- 关窗路径 append 到 `Loom.log`（`LOOM_QUIT_TRACE` 可强制写出），带时间戳与耗时。
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

**不必**先引入完整 log 框架，也 **不必** 先做 Settings。

### Phase 1 — Zed 同款骨架

1. 启动 `init_logger()`：读 Settings（及环境变量覆盖）；TTY → stderr；否则 → 文件。
2. 依赖：`log` + 简单文件 writer（或日后 tracing / 精简 zlog）。
3. 超阈值最大体积 → rename `Loom.log.old` → 新建。
4. 默认级别 `info`；`LOOM_LOG` 可覆盖 Settings。
5. 关键路径用 `log::info!` / `log::warn!`；业务路径不散落裸 `eprintln!`。

### Phase 2 — Settings + 产品体验

- Settings **Logging** 区：级别、目录、启用开关、Open / Reveal。
- `SettingsFile` 增加 `logging` 段（serde 默认兼容旧文件）。
- 改配置后尽量热切换 logger。
- ARCHITECTURE / troubleshooting 写明默认路径与优先级。

（Phase 1 可先只认环境变量 + 默认路径；Phase 2 接上 Settings。）

---

## 原则

| 原则 | 做法 |
| --- | --- |
| 正式跑默认进文件 | 窗卡死时用户仍能打开 `.log` |
| Settings 可配 | 级别、目录、开关；Open / Reveal |
| 环境变量优先 | 临时排障不改持久配置 |
| 默认别吵 | 默认 `info`；禁止每 chunk PTY / 每帧 paint 打 debug |
| 可按模块开大 | 例：`LOOM_LOG=info,loom::ui::persistence=debug`（模块过滤仍以 env 为主；Settings 先做全局级别） |
| 关键路径要阶段名 | `quit.*`、`ssh.connect.*`、`sftp.transfer.*` |
| 日志勿堵 UI | 短消息；大 payload 截断；关窗诊断记 `elapsed_ms`；写盘失败不阻塞 |
| 失败可降级 | 文件失败 → stderr，应用照常启动 |

---

## 建议先覆盖的事件（Phase 1）

| 域 | 示例 | target |
| --- | --- | --- |
| 生命周期 | 启动、`WillQuit`、flush、quit | `loom` / `loom::quit` |
| SSH | connect begin/ready/fail/timeout、auth、host key、disconnect | `loom::ssh.connect` |
| Forward | start/stop/bind fail、AllowTcpForwarding | `loom::ssh.forward` |
| SFTP | channel open、budget、download/upload 起止与失败 | `loom::sftp` / `loom::sftp.transfer` |
| 持久化 | `persist_now` 失败 | `loom::persist` |

**级别建议：** 连接成功/失败、传输起止 → `info`；TCP/auth 阶段、listening → `debug`；host key / bind / transfer 失败 → `warn`。

不要：每次 keystroke、每次 forward `changes` notify、全量终端字节、逐 chunk 进度。

---

## 与现有代码的关系

- 业务诊断统一走 `log` → `Loom.log`（及可选 stderr）；不散落裸 `eprintln!`。
- 配置目录：`src/platform.rs` / `src/platform/windows.rs`（`%APPDATA%\Loom`）。
- Settings UI：`src/ui/settings.rs`；持久化：`SettingsFile`（`src/model/workspace.rs`）。
- 关窗嫌疑：`docs/WINDOW_CLOSE_HANG.md` — Phase 0 专门验证。

---

## 非目标（本设计不做）

- 遥测上报 / 崩溃自动上传
- 按天保留 N 份复杂轮转
- 结构化 JSON 日志（除非日后排障证明需要）
- 在 UI 线程同步写超大文件
- Settings 里做细到每个 crate 的模块过滤器（用 `LOOM_LOG=` 即可）

---

## 实施清单

- [x] Phase 0：`LOOM_QUIT_TRACE` + 关窗阶段日志  
- [x] Phase 1：`init_logger` + 默认路径 + 轮转 + env 覆盖  
- [x] Phase 2：`SettingsFile.logging` + Settings Logging 区（级别 / 目录 / 开关 / Open·Reveal）  
- [ ] 用长会话 Bound Local 复现关窗，对照 `quit: cwd` 行验证 WINDOW_CLOSE_HANG（手工）