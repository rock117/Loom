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
| 5 | ~~`BoundLocalCwdChanged`~~：**已移除**。Bound Local start dir 仅由 Edit Local / Tab **Save** 显式更新（见 [SESSION_PROFILE_IA.md](./SESSION_PROFILE_IA.md) 规则 9） |

## 实体与事件

```text
AppBus  ──emit──►  AppBusEvent
                      │
                      ▼
                 Persistence（subscribe）
                      │
         ┌────────────┼────────────┐
         ▼            ▼            
     WillQuit   PersistRequested  
     flush+quit  debounce flush   
```

### `AppBusEvent`

```rust
pub enum AppBusEvent {
    /// 即将退出：立即全量 flush，然后 quit。
    WillQuit,
    /// 会话 / UI 等需要落盘：合并进 debounce 后 flush。
    PersistRequested,
    // … SplitPane / ReconnectPane / DuplicateActiveTab / Toast …
}
```

`AppBus`：空壳 Entity，`impl EventEmitter<AppBusEvent>`。

### `Persistence`

持有：

- `Entity<AppBus>`（订阅）
- `Entity<WorkspaceStore>`、`WeakEntity<WorkspaceView>`
- debounce 任务句柄
- `flushed_for_quit`（避免重复 quit / 关窗兜底）

行为：

| 事件 | 行为 |
|------|------|
| `PersistRequested` | 调度 debounce（短间隔合并多次 tab/UI 变更） |
| `WillQuit` | 取消 debounce → **立即** `flush_persist_for_quit` → `cx.quit()` |

**Debounce**：约 **300ms**；到时经 `WorkspaceView::flush_persist` 同步 open_tabs / UI 尺寸后 `persist_now`。  
`flush_persist` **不再**把 live shell cwd 写回 Local Profile。

### 关窗

```text
点 X → prepare_window_close → flush_persist_for_quit → return true
Ctrl+Q → WillQuit → flush_persist_for_quit → cx.quit()
```

- **禁止**在 `QuitApp` 里直接 `persist_tabs` + `quit`。
- **禁止**在 UI 线程为写 Profile 而同步 `process_cwd`（关窗卡死历史见 [WINDOW_CLOSE_HANG.md](./WINDOW_CLOSE_HANG.md)）。

## 与现有代码的对应

| 现状 | 第一阶段后 |
|------|------------|
| `QuitApp` → `persist_tabs` → `quit` | `emit(WillQuit)` |
| `persist_tabs()` 多处 | `emit(PersistRequested)` |
| live `cd` → Profile cwd | **不写回**；显式 Edit Local / Save cwd |
| Settings / Profile CRUD 内 `persist_now` | **不变**（非本阶段） |

## 非目标（第二阶段不做）

- 把 Settings / Profile CRUD 的 `persist_now` 全部改为事件
- 全局 EventBus / 字符串 topic
- 后台线程写盘（若 IO 变热再单开）
