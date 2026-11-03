# SFTP 浏览连接 + 独立传输连接

相关文档：[CONTEXT_PANEL.md](./CONTEXT_PANEL.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[SSH_JUMP.md](./SSH_JUMP.md)。

> **文档约定**：此后新增 / 大改的 `docs/*.md` 默认使用**中文**撰写。

## 结论（现行模型）

| 用途 | 连接 |
|------|------|
| 浏览（Home / List / Mkdir / Remove / Rename / Chmod） | 复用该 pane 的 **shell SSH 连接**，开 1 条 SFTP channel |
| 上传 / 下载（含目录、压缩、Docker-over-SSH） | **每个任务新建一条 SSH 连接**，完成后立刻断开 |
| 同一 host 并发传输 | **最多 10 个**，超出排队（Queued） |

一次传输一条连接，互不干扰：一个大目录下载不会堵住 Files 切目录，也不会因为 shell 断线而失败（连接信息在传输开始时已快照）。

## 为什么不再用双车道池

旧方案在 shell 连接上开 Browse + Transfer 两条 channel，并用全局 12 条预算限制。问题：

- 传输挤占 shell 连接的 `MaxSessions`，同时开会话多时更易被服务端拒绝
- 传输与 shell 生命周期绑死，shell 断了传输也跟着断
- 双车道 + 预算 + LRU 回收的状态机难维护，收益有限

现在浏览仍然是「懒开 + 空闲回收」，传输走独立短连接，代码路径简单得多。

## 浏览侧

```
SSH session（每 pane 一条 TCP）
├─ Shell / PTY channel            ← 始终保留（关 tab 才拆）
└─ SFTP channel（0 或 1）         ← 首次 List 才开
```

- 懒开：SSH 连上后默认 **0** 条 SFTP channel
- 空闲回收：**90s** 无请求 → 关 channel、归还全局预算
- 全局预算：最多 **12** 条浏览用 SFTP channel（`GLOBAL_SFTP_CHANNEL_BUDGET`）
- 关 tab：drop `ssh_sftp` → worker 结束 → channel 关闭 → 预算归还
- Inactive / 失焦 **不** 断开 shell

## 传输侧

```
每个任务：
  HostSlot::acquire(host)      ← 同 host 最多 10，超出 200ms 轮询排队
    ↓
  ssh::connect_handles(...)    ← 新建 TCP（有跳板则先连跳板 + direct-tcpip）
    ↓
  request_subsystem("sftp")
    ↓
  download / upload / docker cp 暂存
    ↓
  disconnect + drop slot
```

要点：

| 项 | 行为 |
|----|------|
| 连接复用 | **不复用**；一任务一连接 |
| 跳板 | 沿用 profile 的 `jump`，目标机仍经 `direct-tcpip` |
| 并发键 | `user@host:port`（同机不同账号各自限流） |
| 上限 | `MAX_CONCURRENT_PER_HOST = 10`（`src/session/sftp.rs`） |
| 排队 | UI 状态 **Queued**；取消会立刻退出队列 |
| 结束 | 成功、失败、取消都 `disconnect` 并释放槽位 |
| 线程 | 每个任务一条 `loom-transfer` OS 线程 + 单线程 Tokio runtime |
| 进度 | `bounded(64)` + `try_send`（满则丢中间帧），结果 `bounded(1)` |
| 取消 | 共享 `TransferCancel`（`AtomicBool`），每轮读写前检查 |

连接信息来自 `SshSessionHandles.transfer_target: TransferTarget`，连接成功时快照进
`PaneSession.ssh_transfer_target`；关 pane / 重连时一起 drop。

## 关 Tab / Pane 回收

1. `drop(ssh_sftp)` → 浏览 worker 结束
2. `drop(ssh_transfer_target)` → 之后无法再发起传输
3. 已发起的传输：UI 侧 `cancel` 置位，读到即 `bail!("cancelled")` 并断开自己的连接
4. `ssh_shutdown` → 断开 shell 连接

## 明确不做

- 在 shell 连接上传文件（除浏览用的只读/轻量变更）
- 每 host 常驻连接池 / 长连复用
- 用 shell `ls` 解析替代 SFTP 浏览
- 跨 host 的全局传输并发上限（当前按 host 计）
