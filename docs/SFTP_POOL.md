# SFTP 连接池与文件浏览并行方案

相关文档：[CONTEXT_PANEL.md](./CONTEXT_PANEL.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)。

> **文档约定**：此后新增 / 大改的 `docs/*.md` 默认使用**中文**撰写。

## 背景与问题

当前每个 SSH pane 共用 **一个** SFTP worker + **一个** `SftpSession`，请求**完全串行**：

- 大文件上传/下载进行时，`List`（切目录）会排队，Files 长时间停在 Loading
- 空闲切目录仍慢于 shell：SFTP 必须完整 `readdir`，而 shell 的 `cd` 本身不列目录
- 若每个 SSH 再常驻多条 SFTP channel，开几十个 tab 会顶满本机 FD / 远端 `MaxSessions`

## 目标

1. **同连接上浏览与传输并行**（下载时仍可切目录）
2. **复用同一条 SSH TCP**（`russh` `Handle`），不另开连接
3. **懒开、空闲回收、失焦休眠、关 tab 立刻回收**
4. **全局 channel 预算**，资源不足时降级报错，**绝不拖垮应用 / shell**
5. Inactive 时 **只关 SFTP，不断开 shell**

非目标：用 shell 解析 `ls` 做浏览器；为「和 `cd` 一样快」做假优化。

## 架构总览

```
SSH session（每 pane 一条 TCP）
├─ Shell / PTY channel          ← 始终保留（关 tab 才拆）
└─ SftpPool（逻辑池，默认真 session = 0）
   ├─ Lane::Browse  （最多 1）  ← Home / List
   └─ Lane::Transfer（最多 1）  ← Download / Upload / 目录预计数
```

- **协议**：浏览与传输都走 SFTP（结构化 API，非「只能传文件」）
- **并行**：靠 **多条 SFTP subsystem channel**；单条 `SftpSession` 内部仍串行
- **UI**：全局一个 Context 面板，只绑当前焦点 pane；不因此给每个 tab 常驻满池

## 硬约束

| 约束 | 说明 |
|------|------|
| 不崩 | 开 channel 失败 / 超预算 → 明确错误；shell 继续可用；禁止无限重试 |
| 懒开 | SSH 连上后默认 **0** 条 SFTP channel |
| 空闲回收 | session 无请求超过阈值 → 关 channel，归还预算 |
| 失焦休眠 | pane 失焦且无传输 → 可加速回收 Browse；Transfer 传到完再收 |
| 关 tab | **立刻**回收该 pane 全部 SFTP + 还预算 + 取消 in-flight |
| 不断 shell | Inactive **不**断开 SSH/shell |

## 状态机（每 SSH pane 的 SFTP）

```
Absent / Dormant  →  无真实 channel（可保留轻量 Handle 入口）
       ↓ 首次 List / 传输
Warming           →  正在 open subsystem
       ↓
Active            →  有 Browse 和/或 Transfer session
       ↓ 队列空
Idle              →  开始空闲计时
       ↓ 超时 / 失焦策略
Dormant           →  关掉 channel，归还预算
       ↓ 关 tab / SSH disconnect
Dead              →  不可再开
```

## 全局预算

| 项 | 默认建议 |
|----|----------|
| 全应用同时打开的 SFTP channel 上限 | **12** |
| 单 SSH 同时 SFTP | **≤ 2**（Browse 1 + Transfer 1） |
| Browse 空闲回收 | **60–120s** |
| 失焦且无传输时 Browse | **15–30s** 或立即收 |
| Transfer 空闲回收 | 传完后 **60s**；有进行中任务则不收 |

超限开新 channel 时：先 LRU 回收其它 pane 的 Idle session；仍不够则请求失败（可读错误），不重试打爆。

## 请求路由

| 请求 | 车道 |
|------|------|
| `Home` / `List` | Browse |
| `Download` / `Upload` | Transfer |
| 目录传输前的整树计数 | Transfer（禁止占 Browse） |

进度 channel **有界**（如 64）：满则丢中间进度，保证最终 `TransferOutcome`。

## 关 Tab / Pane 回收清单

1. `ssh_sftp.take()`（丢掉入口，打断后续请求）
2. `pool.close_now()`：关一切 SFTP channel，in-flight → `Err(closed)`
3. 归还 `GlobalSftpBudget`
4. `ssh_shutdown`：断开 SSH（现有路径）
5. Context 面板：若 `bound_pane` 匹配 → 清空浏览状态；剔除相关 transfer
6. drop terminal / PTY（现有）

验收：关 tab 后数秒内无残留 SFTP channel / 池任务；budget 已还。

## 浏览体感

1. **`list_gen`**：快速连点只采纳最新一次 List 结果  
2. **焦点预热（可选）**：仅对当前焦点 SSH 预开 Browse，避免第一次点 Files 才握手  
3. 不做 Browse 路径上的整树扫描  

## 实现落点

| 模块 | 职责 |
|------|------|
| `src/session/sftp.rs` | 池、双车道、懒开、回收、预算租约、`close_now` |
| `src/session/ssh.rs` | 创建池；shutdown 时关池 |
| `src/ui/tab_manager.rs` | teardown 显式 drop handle + 关池 |
| `src/ui/context_panel.rs` | `list_gen`；关 pane 时清状态 |
| 本文档 | 规格来源 |

## 落地顺序

1. 全局 budget + 懒开 + 空闲/失焦回收 + **关 tab 回收**（防崩）
2. 同连接 Browse / Transfer 双车道（防互相堵）
3. List generation +（可选）焦点预热

## 验收标准

- 下载大文件时连续切目录：Loading 不被传输拖死  
- 30～50 个 SSH 只开 shell：SFTP channel ≈ **0**  
- 轮流打开 Files：同时 channel ≤ 全局硬顶  
- Inactive：SFTP 可休眠；**shell 仍在**  
- 关 tab：资源立刻回收，应用不崩  
- 超预算：Files 报错，已有会话正常  

## 明确不做

- Inactive 自动断开 shell（除非将来做可选「省连接模式」且有明确提示）  
- 用 shell `ls` 解析替代 SFTP 浏览  
- 每连接常驻满池（会导致多 tab 时资源爆炸）
