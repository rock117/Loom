# 持久化事件（第一阶段）

相关文档：[GPUI_EVENTS.md](./GPUI_EVENTS.md)、[SESSION_PROFILE_IA.md](./SESSION_PROFILE_IA.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)。

> **状态**：已实施（第一阶段）。  
> **文档约定**：中文；gpui **0.2.x** 实体事件（非全局 EventBus）。  
> **范围**：仅第一阶段。Settings / Profile CRUD 等仍可在 `WorkspaceStore` 内直接 `persist_now`（不做第二阶段收拢）。

## 目标

解耦「业务触发」与「写盘」：

- 触发方只 **emit 事实 / 请求**
- **`Persistence` Entity** 统一订阅并写盘（或改内存 + dirty）
- 关闭应用时 **不** 在 `QuitApp` / 关窗回调里直接 `flush_persist`

## 决策记录

| # | 决策 |
|---|------|
| 1 | 包含 **`PersistRequested`** |
| 2 | 退出语义 **B**：只 `emit(WillQuit)`；在 Persistence 回调里 flush 后再 `cx.quit()`；Ctrl+Q 与关窗同一条路径 |
| 3 | 单独 **`AppBus` Entity** 作为发射方 |
| 4 | 新建 **`Persistence` Entity** 作为写盘监听者 |
| 5 | **`BoundLocalCwdChanged`**：只改内存 + `mark_dirty`；真正落盘靠 `WillQuit` / `PersistRequested` / **定时（debounce）** |

## 实体与事件

```text
AppBus  ──emit──►  AppBusEvent
                      │
                      ▼
                 Persistence（subscribe）
                      │
         ┌────────────┼────────────┐
         ▼            ▼            ▼
     WillQuit   PersistRequested  BoundLocalCwdChanged
     flush+quit  debounce flush   更新 profile cwd + dirty
                                   （并踢一脚 debounce）
```

### `AppBusEvent`

```rust
pub enum AppBusEvent {
    /// 即将退出：立即全量 flush，然后 quit。
    WillQuit,
    /// 会话 / UI 等需要落盘：合并进 debounce 后 flush。
    PersistRequested,
    /// Bound Local 会话 cwd 变化：只更新 store 内存并 mark_dirty。
    BoundLocalCwdChanged {
        profile_id: Uuid,
        path: PathBuf,
    },
}
```

`AppBus`：空壳 Entity，`impl EventEmitter<AppBusEvent>`。

### `Persistence`

持有：

- `Entity<AppBus>`（订阅）
- `Entity<WorkspaceStore>`、`Entity<TabManager>`
- `WeakEntity<WorkspaceView>`（调用现有 `flush_persist`：同步 tabs / Bound cwd / UI 尺寸）
- debounce 任务句柄
- `flushed_for_quit`（避免重复 quit / 关窗兜底）

行为：

| 事件 | 行为 |
|------|------|
| `BoundLocalCwdChanged` | `update_local_profile_cwd`（已有：改内存 + `mark_dirty`，**不** `persist_now`）→ 调度 debounce |
| `PersistRequested` | 调度 debounce（短间隔合并多次 tab/UI 变更） |
| `WillQuit` | 取消 debounce → **立即** `flush_persist` → `cx.quit()` |

**Debounce**：约 **300ms**；到时若仍需要写盘，经 `WorkspaceView::flush_persist` 全量同步后 `persist_now`。  
`WillQuit` 与显式「立刻保存」需求都走立即路径（`WillQuit` 必立即；`PersistRequested` 用 debounce 即可，Ctrl+S 也 emit `PersistRequested`——若需「保存完立刻 toast」可在 debounce 前再立即 flush 一次，实现时 Ctrl+S 采用 **立即 flush**）。

## 谁 emit

| 事件 | 发射方 |
|------|--------|
| `WillQuit` | `WorkspaceView`（`QuitApp`）；窗口 `on_window_should_close`（点 X） |
| `PersistRequested` | `WorkspaceView`（原 `persist_tabs()` 调用点） |
| `BoundLocalCwdChanged` | `TabManager`（收到 `TerminalViewEvent::WorkingDirectoryChanged` 且为 Bound Local 后） |

辅助：`AppBus::emit` 通过 `app_bus.update(cx, \|_, cx\| cx.emit(...))`。

## 退出路径（方案 B）

```text
Ctrl+Q                          窗口标题栏 X
    │                                 │
    ▼                                 ▼
emit(WillQuit)              on_window_should_close:
    │                         emit(WillQuit); return false
    │                         （先挡住销毁，等 flush）
    └────────────┬────────────┘
                 ▼
          Persistence 回调
           flush_persist
           flushed_for_quit = true
           cx.quit()
```

- **禁止**在 `QuitApp` 里直接 `persist_tabs` + `quit`。
- **禁止**把 `observe_release → flush_persist` 当主路径；若保留，仅作 `!flushed_for_quit` 时的兜底。
- `emit` 经 Effect 队列异步派发，故关窗必须用 `should_close` 返回 `false`，不能假设「emit 后立刻已写完再销毁」。

### 已知风险（长会话关窗）

`WillQuit` → `flush_persist` → `bound_local_cwds` 会在 **UI 线程** 同步调用 `process_cwd`。若此处阻塞，则 `cx.quit()` 达不到，窗口因 `should_close == false` **一直不消失**。分析与复现条件见 [WINDOW_CLOSE_HANG.md](./WINDOW_CLOSE_HANG.md)（**尚未改代码**）。

## 与现有代码的对应

| 现状 | 第一阶段后 |
|------|------------|
| `QuitApp` → `persist_tabs` → `quit` | `emit(WillQuit)` |
| `observe_release` → `flush_persist` | `should_close` → `WillQuit`；release 仅兜底 |
| `persist_tabs()` 多处 | `emit(PersistRequested)` |
| TabManager 内 `update_local_profile_cwd` + `persist_now` | emit `BoundLocalCwdChanged`；Persistence 只更新内存 + dirty + debounce |
| Settings / Profile CRUD 内 `persist_now` | **不变**（非本阶段） |

## 非目标（第二阶段不做）

- 把 Settings / Profile CRUD 的 `persist_now` 全部改为事件
- 全局 EventBus / 字符串 topic
- 后台线程写盘（若 IO 变热再单开）

## 实施清单

1. 文档（本文）
2. `app_bus.rs` + `persistence.rs`，接入 `ui` 模块
3. `WorkspaceView::new` 创建并挂住 `AppBus` / `Persistence`；替换 `persist_tabs` / `QuitApp`；注册 `on_window_should_close`
4. `TabManager` 持有 `AppBus`，cwd 路径改 emit
5. `app.rs` 去掉（或降级）`observe_release` 主路径 flush
6. `cargo check`
