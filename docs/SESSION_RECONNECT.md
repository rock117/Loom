# SSH / PTY 断线检测与手动重连

知识笔记 + 已落地行为说明：SSH（及本地 PTY）断开后「假活输入」、只能重新开 tab 的问题，以及当前 **手动重连** 方案。  
**不是自动重连**；自动退避重连若以后要做，见文末「未做」。

相关：[ARCHITECTURE.md](./ARCHITECTURE.md)、[HARD_PROBLEMS.md](./HARD_PROBLEMS.md)、[SFTP_POOL.md](./SFTP_POOL.md)、[LOOM_CLI.md](./LOOM_CLI.md)（`loom reconnect` 规划）。

> **状态**：核心路径 **已实现**（检测 → Failed → 状态栏 Reconnect → 同 tab 重建会话）。  
> **文档约定**：新增规格 / 知识笔记默认中文。

---

## 现象

1. SSH 掉线后终端仍显示旧 prompt，**按键无效果**（像还能输入）。  
2. Context **Files** 常出现 `SFTP unavailable`，与 shell「看起来还活着」不一致。  
3. Tab / 状态点仍可能偏绿（Connected），**状态栏不出现 Reconnect**。  
4. 用户只能关掉 tab、重新打开 profile。

---

## 根因

| 环节 | 问题 |
|------|------|
| 生命周期 | PTY/SSH 读端 EOF 或 channel 关闭后，`pane.state` **未**改为 Failed/Disconnected |
| 写入 | `write_to_pty` 对 `BrokenPipe` **静默忽略**（`let _ = write_all`） |
| UI | 状态栏 Reconnect **仅在** `Disconnected \| Failed` 时显示 → 状态不更新则入口永不出现 |
| 密码重连 | 点 Reconnect 若需密码，原先弹窗提交会走 **新开 tab**，而不是重连当前 tab |
| Files | `sync_session` 只看 pane id，SFTP handle 从有变无/再有时 **不重新绑定** |

终端侧已有 `TerminalEvent::Exit` 与 `with_exit_callback`，但 TabManager 的 `wire_terminal_session` 原先只处理 Focus/Close，**未接 Exit → 会话失败**。

---

## 产品决策

- **主路径 = 可靠检测 + 手动重连**（状态栏 Reconnect），不是自动重连。  
- 断线后 **保留最后一屏输出** 与横幅提示；同 tab 重建 IO，尽量保留分屏/tab 布局。  
- 状态栏 **Reconnect = 重连该 tab 下所有 pane**（每个 split 各自一条会话，全部重建）。  
- 密码认证缺钥匙串时：弹已有 PasswordPrompt，提交后 **reconnect 当前 tab（全部 pane）**。

---

## 已实现方案

### 1. 会话结束事件

`TerminalViewEvent::SessionEnded`：在以下情况触发一次（`session_alive` 门闩）：

- 读任务 EOF → `TerminalEvent::Exit`  
- stdin 写入失败（channel 已断）

### 2. TerminalView

- `session_alive == false` 后：按键 / IME / 粘贴 **不再写入** PTY。  
- 顶部横幅：提示使用状态栏 **Reconnect**。

### 3. TabManager

- 订阅 `SessionEnded` → `on_pane_session_ended`：  
  - `state = Failed`  
  - 文案提示用状态栏重连  
  - 回收 SFTP / SSH shutdown / 本地 PTY  
  - **保留** `terminal` 实体（最后输出 + 横幅）  
- `reconnect` / `reconnect_with_password`：遍历该 **tab 内全部 pane**（含 split），各自 teardown 旧 IO，保留 pane id 拉起 Local 或 SSH（布局不变）。

### 4. WorkspaceView

- `pending_reconnect_tab`：状态栏 Reconnect 且需密码时记下 tab id。  
- PasswordPrompt 提交：有 pending → `reconnect_with_password`；否则仍 `open_ssh_with_password`（首次打开）。

### 5. Context Files

- `bound_sftp_alive`：SFTP 有/无变化时强制 `sync_session`，断线显示 unavailable，重连后重新 `go_home_sftp`。

### 数据流（简图）

```text
SSH/PTY 死
  → Exit 或 write 失败
  → note_session_ended → SessionEnded
  → pane Failed + 回收 SFTP
  → 横幅 + 状态栏 Reconnect
  →（可选密码）→ reconnect_* → 新 terminal / sftp
```

---

## 关键文件

| 路径 | 角色 |
|------|------|
| `src/terminal/gpui_emu/view/mod.rs` | `session_alive`、写失败检测、横幅 |
| `src/terminal/gpui_emu/view/context_menu.rs` | `SessionEnded` 事件 |
| `src/ui/tab_manager.rs` | 订阅结束、`reconnect*` |
| `src/ui/workspace_view.rs` | 密码重连 pending |
| `src/ui/status_bar.rs` | Failed 时显示 Reconnect（原有） |
| `src/ui/context_panel.rs` | SFTP 存活变化重绑 |

---

## 验收要点

- [ ] 远端断 SSH / 杀会话后：横幅出现，状态 Failed，Reconnect 可见。  
- [ ] 断线后按键不产生「假输入」；重连成功后可正常输入。  
- [ ] 密码 profile（无钥匙串）走 Reconnect → 输密码 → **同一 tab** 恢复，不另开一个。  
- [ ] **Split 多 pane**：断线后点 Reconnect，该 tab 内所有分屏都进入 Connecting 并恢复（不只焦点 pane）。  
- [ ] Files：断线 unavailable；重连后可再列目录。  
- [ ] Local shell 退出同样 Failed + 可 Reconnect。

---

## 未做（有意后置）

| 项 | 说明 |
|----|------|
| 自动重连（退避、次数上限） | 需处理密码、误重连、抢焦点；产品上另议 |
| SFTP 单独 Retry（不断 shell） | 半开连接时有价值；当前整会话 Reconnect 已覆盖主痛点 |
| 终端内横幅上的 Reconnect 按钮 | 现依赖状态栏；可后续加事件 |
| `loom reconnect` 壳内指令 | 见 [LOOM_CLI.md](./LOOM_CLI.md) |

---

## 规则（给后续改动）

1. 会话 IO 死亡必须反映到 `ConnectionState`，不能只留死 writer。  
2. 对 PTY 的写失败不能静默成功。  
3. Reconnect 与「新开 profile」必须分清（密码回调尤其容易混）。  
4. Files 绑定不能只靠 pane id，还要感知 SFTP handle 生死。
