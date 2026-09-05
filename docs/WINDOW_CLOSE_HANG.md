# 多 Tab / 长会话：点 X 窗口不消失（原因分析）

相关文档：[PERSISTENCE_EVENTS.md](./PERSISTENCE_EVENTS.md)、[HARD_PROBLEMS.md](./HARD_PROBLEMS.md)、[SFTP_POOL.md](./SFTP_POOL.md)、[LOGGING.md](./LOGGING.md)。

> **状态**：原因分析（**尚未改代码**）。  
> **日期**：2026-09-05。  
> **文档约定**：中文。

---

## 一句话

点标题栏 X 后窗口不销毁，是因为 **方案 B 先 `should_close → false` 挡住关窗**，真正退出依赖 **`WillQuit` 里同步 `flush_persist` 再 `cx.quit()`**；flush 若在 UI 线程上卡住（长会话下最可疑的是 **Bound Local 的 `process_cwd` / sysinfo**），窗口就会一直停在「已点 X、却关不掉」。

---

## 复现画像（已确认）

| 项 | 内容 |
| --- | --- |
| 症状 | 点窗口 **X**，**窗口不消失**（不是「窗没了进程还在」） |
| Tab | **Local + SSH 混开** |
| 会话时长 | Loom **连续用近 1 天**未关过 |
| 伴随操作 | 有过 SFTP **上传/下载**；开过终端 **查找框** |
| 版本 | 卡住时为 **转发功能落地前**（约 2026-09-04 20:00 前），**无端口转发** |
| 对照实验 | **连开 6 个 tab 立刻关** → **秒关**，不复现 |

对照实验很重要：说明不是「tab 数量本身」或「刚连上时的瞬时风暴」，而是 **长跑之后的退出路径** 出问题。

---

## 关窗协议（方案 B）在做什么

见 [PERSISTENCE_EVENTS.md](./PERSISTENCE_EVENTS.md)「退出路径」。

```text
用户点 X
    │
    ▼
on_window_should_close
    │  flushed_for_quit? ──yes──► return true  → 系统销毁
    │
    no
    │
    ├─ emit(WillQuit)     （经 Effect 队列，非「emit 完已写盘」）
    └─ return false       ← 窗口先留下
         │
         ▼
    Persistence::on_will_quit
         flush_persist     ← 同步，UI 线程
         flushed_for_quit = true
         cx.quit()         ← Windows：PostQuitMessage(0)
```

关键代码：

- `src/ui/workspace_view.rs` — `on_window_should_close`
- `src/ui/persistence.rs` — `on_will_quit` / `flush_now`
- GPUI Windows：`platform.quit()` → `PostQuitMessage`（**不会**再走一遍「成功的 WM_CLOSE」）

因此：

- **`return false` 之后，若 `WillQuit` 未跑完或 `flush` 不返回，窗口可以永久不消失。**
- 这与「连开秒关」不矛盾：短会话时 flush 很快，`quit` 能跟上。

---

## 已排除的假设

| 假设 | 为何排除 |
| --- | --- |
| 端口转发 `ForwardHandle::start` 在 UI 上 `recv_timeout`（≤8s/条） | 复现版本 **尚无转发**；且「连开立刻关」秒关 |
| 「刚开多 tab、转发/连接回调堵 UI」时序 | 用户对照：**连开 6 个秒关** |
| 查找框 caret blink / `RenameEdit` | 周期 timer + `notify`，取消即停；无法解释「仅长会话点 X 窗不走」为主因 |
| 「窗没了但进程残留」类问题 | 用户明确是 **窗口不消失** |

> 注：当前 main 上的 `forwards.start` 同步等待仍是 **独立的 UI 冻结风险**（多 SSH + Profile 转发时），与本次长会话关窗事故 **不是同一条因果链**；修复关窗时不必绑在一起，但 checklist 仍应单独盯。

---

## 主嫌疑（高）：WillQuit → `flush_persist` → `process_cwd`

### 调用链

```text
Persistence::on_will_quit
  → WorkspaceView::flush_persist
    → TabManager::bound_local_cwds
      → 每个 Bound Local pane：
           terminal.refresh_working_directory()
             → platform::process_cwd(shell_pid)   // sysinfo，UI 线程同步
    → WorkspaceStore::persist_now()               // 写 JSON，一般可完成
```

相关代码：

- `src/ui/tab_manager.rs` — `bound_local_cwds`
- `src/terminal/gpui_emu/view/context_menu.rs` — `refresh_working_directory`
- `src/platform.rs` — `process_cwd`（`sysinfo` + `ProcessRefreshKind::cwd`）

### 为何贴合「用一天再卡」

1. **仅 Bound Local**（侧栏绑定、有 `profile_id`）会走 `process_cwd`；纯临时 Ctrl+T / 纯 SSH 不走这条。混开时往往有 Bound Local。
2. **长寿命 ConPTY / shell**：Windows 上对个别进程读 PEB cwd，偶发 **长时间不返回或极慢**；新开的壳在「连开秒关」里通常正常。
3. 卡在 flush → **到不了 `cx.quit()`** → `should_close` 一直是 false 语义下的「窗还在」→ 与症状一致。
4. CPU 常接近空闲（堵在系统调用），符合 [HARD_PROBLEMS.md](./HARD_PROBLEMS.md)「Idle CPU + frozen UI ≈ 阻塞等待」。

### 尚未在调试器里钉死

本文是 **代码路径 + 复现条件** 的推断。下次复现时应用调试器看 UI 线程是否停在：

- `sysinfo` / `process_cwd` / `refresh_working_directory`
- 或 `flush_persist` / `persist_now`（盘/杀毒锁文件，次优先）

快速对比：

- 只留 SSH、关掉所有 Bound Local 再点 X → 若不再卡，则 cwd 路径坐实。
- 卡住时任务管理器 Loom CPU ≈ 0 → 优先查阻塞而非忙等。

---

## 次嫌疑：方案 B 在 Windows 上偏脆

即使 flush 最终返回：

1. `should_close` 曾 `false` → 原生 `DestroyWindow` 未走。
2. `cx.quit()` → `PostQuitMessage` → 消息循环退出 → `App::shutdown` → `windows.clear()`。
3. GPUI Windows 的 `WindowsWindow::Drop` 把 `DestroyWindow` **丢到 foreground executor 异步跑**；若循环已退出，该任务可能 **再也跑不到**，依赖进程退出清 HWND。

长会话下若 shutdown / Drop 路径再被其它工作拖住，会加重「窗还挂着」的体感。这是协议层弱点，不是本次唯一根因，但是 **放大因子**：任何卡在 `WillQuit` 之前的阻塞都会变成「窗永不关」。

---

## 次嫌疑：SFTP 半死传输（偏后台）

有过上传/下载、长跑一天，可能存在：

- Transfer lane 卡在 `read`/`write`（网络半开）
- `run_sftp_worker` 在 handle drop 后 **`await` browse/transfer task** 直到传完
- `disconnect` 与仍占着的 SFTP channel 互相拖

这主要占 **SSH Tokio 线程**，一般 **不直接** 挡住 `should_close` 回调。更符合「退出后进程收尾脏 / 变慢」，对「点 X 窗立刻不消失」是 **次要**。关窗/teardown 时强制 cancel、避免无限 `await`，仍建议作为后续加固。

查找框：与长会话同时出现，**不视为根因**。

---

## 因果排序（当前判断）

```text
1. 主：WillQuit 同步 flush 里 process_cwd（长寿命 Local）堵 UI
         → quit 不到 → 窗因 should_close false 留下
2. 放大：方案 B + Windows PostQuitMessage / DestroyWindow 异步
3. 次：卡住的 SFTP await（后台）；查找框（基本无关）
4. 另册：forwards.start 同步 recv（本事故版本无；仍是独立冻结类）
```

---

## 建议修复方向（仅记录，本次不改代码）

1. **WillQuit / `bound_local_cwds`**：退出路径 **不要** 再调 `process_cwd`；用内存里已有 cwd（OSC / 上次刷新）即可。
2. **关窗协议**：flush 与关窗解耦（超时、后台 flush、或先允许关窗再尽力写盘）；避免「false 之后永远等不到 quit」。
3. **SFTP teardown**：关窗/关 tab 时 set cancel，worker 对 in-flight 传输设超时或 `select!` 取消，禁止无限 `await`。
4. **（独立）转发**：`start`/`stop`/`retry` 移出 UI 线程同步 `recv_timeout`。

### 排障日志（可选，先于或并行于修复）

按 [LOGGING.md](./LOGGING.md) **Phase 0**：`LOOM_QUIT_TRACE=1` 时在关窗路径打 `quit: should_close` / `will_quit` / `cwd … begin|ok` / `cx.quit`。若停在 `cwd … begin` 则坐实本分析主嫌疑。

验证计划（改完后）：

- [ ] Bound Local 开一整天（或人为挂起 shell）后点 X，窗应消失
- [ ] 混开 SSH + 进行中/卡住的传输时点 X，窗应消失；进程可短暂收尾
- [ ] 连开多 tab 立刻关仍秒关
- [ ] Ctrl+Q 与点 X 行为一致且会落盘（或明确「尽力写盘」语义）

---

## 代码索引

| 区域 | 路径 |
| --- | --- |
| 关窗拦截 | `src/ui/workspace_view.rs` (`on_window_should_close`) |
| WillQuit | `src/ui/persistence.rs`, `src/ui/app_bus.rs` |
| flush + cwd | `src/ui/workspace_view.rs` (`flush_persist`), `src/ui/tab_manager.rs` (`bound_local_cwds`) |
| process cwd | `src/platform.rs` (`process_cwd`), `src/terminal/gpui_emu/view/context_menu.rs` |
| pane teardown | `src/ui/tab_manager.rs` (`teardown_pane_io`, `TabManager::Drop`) |
| SFTP worker 收尾 | `src/session/sftp.rs` (`run_sftp_worker` 末尾 `await` lanes) |
| 退出文档 | [PERSISTENCE_EVENTS.md](./PERSISTENCE_EVENTS.md) |
| 冻结 checklist | [HARD_PROBLEMS.md](./HARD_PROBLEMS.md) |
