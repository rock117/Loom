# PTY 输出合并处理（coalesce + 单次 notify）

知识笔记 + 已落地行为：高吞吐 PTY 输出时减少 UI `notify`/重绘次数，**不丢弃任何字节**。

相关代码：`src/terminal/gpui_emu/view/mod.rs`（`TerminalView::new` 的 reader 任务、`read_stdout_blocking`）。  
决策记录：[DECISIONS.md](./DECISIONS.md)（2026-09-02）。  
冻结清单：[HARD_PROBLEMS.md](./HARD_PROBLEMS.md) §B。

---

## 问题

读线程按 ≤4KB 一块 `send` 进 `flume::unbounded`；UI 异步任务原先 **每块** 一次：

`process_bytes` → `dispatch_pending_events` → `cx.notify()`

`yes` / `cat` 大文件 / 爆量构建日志时，chunk 速率远高于帧率 → 队列堆积、每帧工作过重、窗口假死。  
根因是 **notify/重绘次数 ≈ chunk 数**，不是读线程丢数据。

---

## 为何原先这么写

| 选择 | 意图 |
| --- | --- |
| 后台阻塞读 + flume push | 有数据才醒 GPUI，避免轮询；flume 与 executor 无关 |
| `unbounded` | **永不因队列满丢 PTY 字节**（丢字节 = 花屏 / VT 错乱） |
| 每 chunk 立刻 `notify` | 实现简单、低延迟 |

正确性优先于洪水下的帧率；副作用是高负载时 UI 被重绘淹没。

---

## 方案（已采纳）

收到第一块后：

1. `try_recv` 抽干通道里**已到达**的后续块  
2. 按序拼成一块 buffer（或等价地按序喂入）  
3. **一次** `process_bytes`  
4. **一次** `dispatch_pending_events`  
5. **一次** `cx.notify()`

通道仍为 **unbounded**；读侧仍 `send` 全部读到的字节。

### 明确不做

- 丢弃 / 采样输出  
- 有界队列满了就 `try_send` 失败丢包（背压阻塞 `send` 可作为后续，仍不丢字节）  
- 把 VTE/`process_bytes` 挪到非 UI 线程（事件回写顺序绑在 view 上）

---

## 会不会丢输出？

**不会。** 丢掉的是多余重绘，不是字节。

| 环节 | 丢吗 |
| --- | --- |
| `read` / `send` | 否 |
| drain `try_recv` | 否（多拿，不跳过） |
| `process_bytes` | 否（顺序完整喂入） |
| 合并 `notify` | 只少画中间帧；最终 grid 与逐块刷完一致 |

可能观感：洪水时画面更「跳」、单次 update 更长，但总忙时间通常更短，更不易假死。

---

## 事件与顺序

`PtyWrite` / OSC cwd / 剪贴板等仍在同一次 `process_bytes` 之后由 `dispatch_pending_events` 按队列顺序处理；字节顺序不变则语义与改前一致。

---

## 验收

- [ ] `yes` 或 `cat` 大文件时窗口仍可拖动、可输入  
- [ ] 正常交互（vim、进度条、彩色输出）不错乱  
- [ ] 断线 / Exit、cwd（OSC 7）、标题、响铃、DSR/`PtyWrite` 仍正常  

---

## 后续（未做）

- 有界 flume + 阻塞 `send`（背压，仍不丢包）  
- Find 防抖 / `search_next`（见稳定性审查；与本文独立）
