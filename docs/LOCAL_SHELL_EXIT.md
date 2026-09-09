# 本地 Shell 退出被当成「断线」

相关：[SESSION_RECONNECT.md](./SESSION_RECONNECT.md)、[LOCAL_SHELL.md](./LOCAL_SHELL.md)、[HARD_PROBLEMS.md](./HARD_PROBLEMS.md)。

> **状态**：根因排查中；产品侧已做文案区分 + 本地有限次自动重启。  
> **范围**：仅 **Local / WSL**；SSH 真断线仍走 Failed + 手动 Reconnect。  
> **文档约定**：规格与排障说明默认中文。

---

## 现象

1. 本地 pane（尤其 **split 出来的那一侧**）顶部出现红色横幅：  
   `Disconnected — click Reconnect…`（后改为本地文案 `Shell exited…`）。
2. 屏幕上仍留着最后一屏（含 prompt、`git pull` 输出等），看起来「还活着」。
3. 同 tab 另一侧（常为最初打开、正跑长任务的 pane）往往正常。
4. 用户体感：本地不该「掉线」；其它分屏终端很少这样晾着。

---

## 为何容易误解

| 误解 | 实际 |
|------|------|
| 只有 SSH 才会 Disconnected | Loom 对 **任意** pane 的 `session_alive == false` 都画横幅（Local/WSL/SSH 共用） |
| 还能看见 prompt = shell 还活着 | 断线后 **保留最后一屏**；按键已不再写入 PTY |
| Split = 同一个 shell 的两个视图 | 每个 leaf 是 **独立** Local PTY / 进程 |
| 不 split 就永远不会出现 | 单 pane 本地仍可能 `exit` / 崩溃 / EOF·写失败；只是更少见 |

---

## 机制（代码路径）

```text
PTY 读 EOF  或  write_to_pty 失败
  → TerminalView::note_session_ended
  → session_alive = false + 横幅
  → TerminalViewEvent::SessionEnded
  → TabManager::on_pane_session_ended
  → teardown_pty（可能杀掉仍存活的进程）+ Failed / 或本地自动重启
```

关键文件：

| 路径 | 角色 |
|------|------|
| `src/terminal/gpui_emu/view/mod.rs` | `session_alive`、写失败、横幅 |
| `src/terminal/gpui_emu/event.rs` | `Exit` / `ChildExit` → 会话结束 |
| `src/ui/tab_manager.rs` | split 再 spawn、`on_pane_session_ended`、本地自动重启预算 |
| `src/session/local.rs` | ConPTY spawn / teardown |

Split 本地时：`split_pane_inner` → `spawn_local(..., profile_id: None, live_cwd)`，**新开一颗** shell，与源 pane 不共用 stdin/stdout。

---

## 为何常是 split、不是原 pane

- 原 pane 常挂长任务（如 `npm run dev`），进程明显活跃。
- Split 侧多为空闲交互壳，跑完命令回到 prompt；任一误判或真退出更显眼。
- 分屏瞬间两侧 resize；新 ConPTY 刚拉起，在 Windows 上更易撞到 EOF/写失败类问题（**待日志坐实**）。

**不 split ≠ 免疫**：单 pane 本地同样走上述路径。

---

## 与其它终端对比

| | 本地 shell 进程退出后 |
|--|----------------------|
| Windows Terminal / VS Code / WezTerm / iTerm 等 | 关 pane、或 “Process exited”、或设置「退出时重启」 |
| Loom（问题暴露时） | 保留缓冲 + SSH 风味断线横幅 + 状态栏 Reconnect |

「进程会退出」各端都有；「本地像 SSH 掉线一样晾着」不是分屏终端通病，是 Loom 把 Local 接进了 SSH 重连 UX。

---

## 已做 / 未做

**已做**

- 本地横幅 / 状态栏文案与 SSH 区分（`Shell exited` / `Exited`）。
- Local/WSL：`SessionEnded` 后有限次静默同 pane 重启（防崩循环预算）；耗尽再 Failed + 手动 Reconnect。
- SSH：仍手动 Reconnect（密码/密钥）。

**未做（根因）**

- 区分并记录触发原因：读 EOF vs `write` 失败 vs（若有）误映射的 Exit。
- 确认 Windows ConPTY + split resize 是否存在假 EOF/假写失败。
- 评估写失败是否过激（一次 `write_all` err 即 `note_session_ended` + teardown）。

---

## 排查时建议收集

1. 出横幅的是 **split 侧** 还是单 pane？  
2. 之前是否还能输入？一 split 就挂还是跑完某命令才挂？  
3. 默认 shell（cmd / pwsh / bash/zsh）与是否刚 resize。  
4. 加日志后：`note_session_ended` 的调用栈/原因标签。

---

## 规则（给后续改动）

1. Local 结束态文案禁止再写成纯 “Disconnected”（易当成 SSH）。  
2. Local 默认应静默恢复或干净结束；不要长期假活缓冲 + 强制手点 Reconnect（除非预算耗尽或用户关闭自动重启）。  
3. 假阳性排查未完成前，不要用 teardown 掩盖「检测是否过严」——误判会把活 shell 杀掉并留下迷惑性 prompt。  
4. Split 必须继续保证 **一 leaf 一会话**；修复不得改成共享 PTY。
