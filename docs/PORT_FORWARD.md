# SSH 端口转发（Local / Remote / SOCKS）

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[BACKLOG.md](./BACKLOG.md)、[SFTP_POOL.md](./SFTP_POOL.md)。

> **状态**：规格已定，**尚未实现**。实现须由用户明确点名后开始。  
> **产品定位**：**增强能力，非核心**。主线仍是终端 + SSH + Files；转发做成 Profile / 会话上的可靠附件即可。  
> **文档约定**：新增规格默认中文。

## 一句话目标

在 **已有 SSH 会话**（与 shell / SFTP 同一条 `russh` 连接）上提供等价于 OpenSSH 的端口转发：

| 类型 | 命令行等价 | 用户体感 |
|------|------------|----------|
| **Local** | `ssh -L` | 连本机端口 → 经跳板到达远端可达的 `host:port` |
| **Remote** | `ssh -R` | 连服务器上的端口 → 打到本机某服务 |
| **SOCKS** | `ssh -D` | 本机 SOCKS5 代理，目标由客户端动态指定 |

**不**另开一条只做隧道的 SSH（除非日后产品明确要求）；**不**在远端部署 Loom 组件——对端只需正常 `sshd`。

## 背景（业界做法）

日常用法是本机一条 `ssh -L/-R/-D`（或 `~/.ssh/config` 里的 Forward），GUI 客户端把同样规则做成会话配置。远端一般**不必**再跑「开启转发」命令。Loom 走 GUI 客户端这条路：连上 Profile 后自动起规则，用户用 DBeaver / 浏览器等连本机端口即可。

## 产品原则

1. **非核心**：无转发时，SSH + Files 主路径完整。  
2. **同连接复用**：与 shell、SFTP 共享 `Handle`；关 Tab / 断线时回收 listener 与桥接任务。  
3. **默认安全**：本机监听默认 `127.0.0.1`，不默认 `0.0.0.0`。  
4. **错误可读**：端口占用、`AllowTcpForwarding no`、Remote 被拒 / 只绑服务器 loopback 等，用明确文案，不静默失败。  
5. **分期**：先 Local，再 Remote，最后 SOCKS。

## 三种转发（协议要点）

### Local（`-L`）— MVP 优先

```
本机 TcpListener(bind)
  → accept
  → channel_open_direct_tcpip(目标 host, port, …)
  → copy_bidirectional(tcp, ssh channel stream)
```

- **每个**本机 TCP 连接对应 **一条**新的 `direct-tcpip` channel。  
- 典型：`本机 15432 → 127.0.0.1:5432`（经 jump 访问远端数据库）。

### Remote（`-R`）— 第二期

```
客户端 tcpip_forward(服务器 bind, port)
  → 对端有人连进来
  → Handler::server_channel_open_forwarded_tcpip
  → 本机 connect(本地目标) → copy_bidirectional
停止：cancel_tcpip_forward
```

- 依赖扩展现有 russh `Handler`（今日多半只做 host key）。  
- 常受 `sshd` 的 `AllowTcpForwarding` / `GatewayPorts` / 安全组影响；UI 需说明「外网未必能连服务器该端口」。

### SOCKS（`-D`）— 第三期

- 本机实现（或引入）SOCKS5：**解析目标**后，底层仍是 **`direct-tcpip`**（与 Local 同族）。  
- 工程量大于固定目标的 Local。

## 架构草图（实现时）

```
SSH pane（同一 russh Handle）
├─ Shell / PTY
├─ SftpPool（Browse / Transfer）
└─ ForwardHub
   ├─ Local listeners…
   ├─ Remote registrations…（二期）
   └─ SOCKS listeners…（三期）
```

硬约束（对齐 SFTP 池经验）：

| 约束 | 说明 |
|------|------|
| 不堵 UI | accept / copy / 开 channel 全在 async，禁止在 render/点击路径里阻塞 |
| 关 Tab | 立刻停 listener、取消 in-flight copy、Remote 则 cancel forward |
| 断线 | 转发随会话结束；自动重连是否恢复规则 → 后期再定（MVP 可不做） |
| Channel 预算 | 活跃 TCP 会占 SSH channel；与 SFTP 共享压力，需上限或可观测，避免拖垮 shell |

## 数据模型（建议）

```text
ForwardRule {
  id
  kind: Local | Remote | Socks
  bind_host, bind_port     // 监听侧
  target_host, target_port // Local/Remote；Socks 无固定目标
  enabled
  status: Idle | Listening | Error(message)
  // 可选：bytes_up / bytes_down / active_connections
}
```

### 挂载位置

| 位置 | 用途 |
|------|------|
| **SSH Profile** | 持久化；连接成功后自动启用 `enabled` 规则 |
| **当前会话** | 临时规则；随 pane 销毁（可不写盘） |

Info / 会话摘要可只读列出本 pane 活跃转发（端口 + 状态）。

## UI（克制）

- Profile 编辑：Port forwarding 列表 + Add（类型、bind、target）。  
- 已连接：启停单条、看 `Listening` / 错误；可选「添加临时转发」。  
- **不做**独立「隧道专用连接」产品线（与主 SSH Tab 合一）。

## 服务端要求

| 模式 | Loom 之外 |
|------|-----------|
| Local / SOCKS | 通常只需远端 `sshd` 允许 TCP forwarding；**无需**装 Loom 或额外命令 |
| Remote | 同上，且更常被 `GatewayPorts`、防火墙限制；属运维配置，非 Loom 后端 |

禁用 `AllowTcpForwarding` 时客户端无法单方面绕过——如实报错即可。

## 非目标（当前）

- 自研非 SSH 的穿透协议（ngrok / frp 类）  
- 单独的「只转发、无 shell」连接类型（可日后评估）  
- Unix domain socket 转发（russh 有能力，不进 MVP）  
- 把转发做成产品核心卖点或首页一级导航  

## 分阶段

| 阶段 | 内容 | 状态 |
|------|------|------|
| 0 | 本规格文档 | **完成** |
| 1 | Local：会话级启停 + Profile 自动启用；默认 bind `127.0.0.1` | 未做 |
| 2 | Remote：Handler + registry；错误文案 | 未做 |
| 3 | SOCKS5；可选流量 / 连接数 | 未做 |
| 4 | 断线重连后恢复规则、与 jump host 组合打磨 | 未做 |

## 实现映射（落地时填写）

| 块 | 预期位置 |
|----|----------|
| 规格 | `docs/PORT_FORWARD.md`（本文） |
| ForwardHub | `src/session/`（新建 forward 模块，挂在 SSH pane） |
| Handler 扩展 | `src/session/ssh.rs`（Remote 回调） |
| Profile 字段 | profile / workspace 模型增加 `forwards` |
| UI | Profile 编辑 + 会话状态（Info 或小面板） |
| Teardown | 与 `tab_manager` 关 pane 路径对齐 SFTP 回收 |

## 验收（阶段 1）

1. SSH 连上后，Profile 中启用的 Local 规则在本机指定端口 Listening。  
2. 本机客户端连该端口，可访问配置的远端目标（如数据库）。  
3. 关 Tab 后端口释放，无残留 listener。  
4. 端口占用或服务器拒绝时有明确错误，UI 不卡死。  
5. 与同时进行的 SFTP 浏览/传输可共存（压力下可降级报错，但不拖死 shell）。

## 如何开做

用户明确说「做端口转发 / 实现 PORT_FORWARD / 做 Local 转发」等之后，再按阶段 1 切片；不要因讨论或「继续 roadmap」自动开工。
