# SSH 端口转发（Local / Remote / SOCKS）

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[BACKLOG.md](./BACKLOG.md)、[SFTP_POOL.md](./SFTP_POOL.md)。

> **状态**：规格已定（含使用场景与 UI），**尚未实现**。实现须由用户明确点名后开始。  
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
5. **侦测转发权限**：实现时须能判断（或在首次失败时明确归因）远端是否允许 TCP forwarding，并在 UI 说明；禁止只显示含糊的 “failed”。  
6. **分期**：先 Local，再 Remote，最后 SOCKS。

## 与跳板的区别（勿混为一谈）

| | **跳板 / Jump** | **端口转发（本文）** |
|--|-----------------|----------------------|
| 解决什么 | **怎么连到目标机**（shell 落在哪） | **连上之后**，怎么把端口借给本机/远端工具 |
| 典型 | `笔记本 → 堡垒 → 内网机` | `本机 :15432 → 经 SSH → 远端 DB :5432` |
| 关系 | 可组合：先 Jump 登录，再在最终会话上开 Local | 不是 Jump 的子集 |

详见产品讨论纪要式说明见下文「使用场景」；Jump 单独排期，不在本文实现范围内。

---

## 使用场景

**一句话：** SSH 负责「进机器」；转发让本机浏览器、数据库 GUI、IDE **经同一条加密连接** 使用只暴露在 SSH 后面的端口。

用户画像偏 **半脚终端、半脚本机图形工具**（后端 / 运维 / 全栈联调），不是纯 shell 刚需，但是连接客户端里粘性很高的能力。

### 1. 远端数据库给本机 GUI（最高频）

Postgres / MySQL / Redis 只监听服务器 `127.0.0.1` 或仅内网可达。本机 DBeaver、DataGrip、TablePlus 无法直连公网。

**Local：** `本机 localhost:15432 → 服务器 127.0.0.1:5432`  

Loom 连上 SSH → 规则 Listening → GUI 填 `127.0.0.1:15432`。终端照常敲命令，GUI 走隧道；关 Tab 端口释放。

### 2. 内网 Web / 管理后台

K8s dashboard、Jenkins、内网文档、仅绑在应用机上的 `8080`；不想为看一眼开公司 VPN。

**Local：** `本机 :8443 → 10.0.1.20:443`，或 `本机 :8080 → 127.0.0.1:8080`（经 SSH 落在服务器本机服务）。浏览器打开本机端口即可。

### 3. 本机前端联调远端 API

API 只在测试机；本地 `npm run dev` 希望请求「像在机房」。Local 映到本机端口，前端 `.env` 指 `http://localhost:3xxx`。终端看日志、转发保活，断线则端口掉，避免隧道残留。

### 4. 与跳板组合碰「再后面」的服务

只能 SSH 到堡垒，数据库在 `10.0.0.8`。Jump 决定 shell 落点；Local 的 target 填内网 DB（或先登到能碰 DB 的机器再转发）。**两种需求，常一起出现。**

### 5. 临时应急（不改防火墙）

线上只开 22，需要短时间用本机工具碰业务端口。Profile 勾选规则，或会话内 **Temporary** 加一条；用完关 Tab。适合排障、导出、跑一次客户端迁移。

### 6. Remote：远端回调本机（次常见）

本地 webhook / 演示用前端，要让测试环境或服务器打到笔记本。

**Remote：** `服务器 :9000 → 本机 localhost:3000`  

常受 `GatewayPorts`、安全策略限制；成功时也扩大暴露面。产品上作进阶，默认强调 Local。

### 7. SOCKS：浏览器「进内网逛一圈」

内网域名/端口很多，不想为每个服务配一条 Local。

**SOCKS：** `socks5://127.0.0.1:1080`，浏览器走代理访问 `http://wiki.corp/...`。  

适合临时巡检；单一 Postgres 仍应用 Local（更简单、攻击面更小）。

### 8. 何时不需要转发

| 你只需要… | 要不要转发 |
|-----------|------------|
| 登录敲命令、看日志、vim | 通常不要 |
| 本机 GUI 连远端 DB / Redis | **要 Local** |
| 本机浏览器看内网站点 | **要 Local 或 SOCKS** |
| 远端回调你本机服务 | **要 Remote** |
| 文件上下传 | Files / SFTP，不是转发 |

### 9. 完整用户故事

Profile `prod-api` 预置：`Local 127.0.0.1:15432 → 127.0.0.1:5432`（备注 Postgres）。

1. 打开 Loom，连接该 Profile。  
2. 状态栏出现 `⇄ 1`，Context → Info 显示 Listening。  
3. DataGrip 连 `localhost:15432`；另一 pane 看日志 / 跑迁移。  
4. 关 Tab：15432 释放。  
5. 翌日加一条 Temporary `8080→内网管理台`，不写盘，用完即弃。

### 10. 明确不承诺的场景

- 代替公司 VPN 做全员网络准入。  
- 服务器 `AllowTcpForwarding no` 时硬开（只能报错说明）。  
- 非 SSH 的公网穿透（frp / ngrok 类）。

---

## UI / 交互设计

### 设计原则

1. **附着会话**：转发是 SSH Tab/pane 的附件，不另开「只隧道」连接类型。  
2. **两处入口**：改持久规则 → Profile 编辑；看状态 / 临时加一条 → 当前会话。  
3. **默认安全**：本机监听默认 `127.0.0.1`；`0.0.0.0` 仅 Advanced + 警告。  
4. **克制**：MVP 只做 Local；列表 + 启停 + 报错；不做流量大盘。  
5. **失败隔离**：单条规则 Error **不**把整条 SSH 标成 Failed。

### Profile 编辑（持久规则）

SSH 表单增加可折叠一节 **Port forwarding**（默认收起）：

```text
Port forwarding                          [+ Add]
┌─────────────────────────────────────────────────────┐
│ ☑ Local   127.0.0.1:15432  →  127.0.0.1:5432   ⋯  │
│ ☐ Local   127.0.0.1:8080   →  10.0.0.5:80     ⋯  │
└─────────────────────────────────────────────────────┘
```

| 元素 | 行为 |
|------|------|
| 勾选 | `enabled`；连接成功后自动启用 |
| ⋯ | Edit / Duplicate / Delete |
| Add | 小表或行内展开（非整页向导） |

**Add / Edit 字段（MVP）：**

| 字段 | 说明 |
|------|------|
| Type | MVP 仅 Local；Remote / SOCKS 二期再露出 |
| Listen | host + port（host 默认 `127.0.0.1`） |
| Target | host + port（经 SSH 可达；`127.0.0.1:5432` = **远端本机**服务） |
| Name | 可选备注，如 `Postgres` |

辅助文案一行：「Connect 成功后自动监听；本机用 `localhost:<port>` 访问。」

### 会话态：Context → Info

右栏 Info 挂运行时列表（不另开「Tunnels」Tab，除非规则长期很多）：

```text
Port forwarding
  ● Postgres    127.0.0.1:15432 → 127.0.0.1:5432    Listening
  ●             127.0.0.1:8080  → 10.0.0.5:80       Error: address in use
  [+ Temporary]
```

| 操作 | 行为 |
|------|------|
| 行 / 状态点 | 启停该条（不停 SSH） |
| Temporary | 仅当前 pane；关 Tab 丢弃；可不写 Profile |
| 错误行 | hover / 展开短原因；可 Retry |
| SSH 未连接 | 灰显：「Connect to enable forwards」 |
| Connecting | 「Waiting for connection…」 |

### 状态栏

与连接状态同一级、极轻：

| 显示 | 含义 |
|------|------|
| （无） | 无 Listening 且无 Error |
| `⇄ N` | N 条 Listening |
| `Forward error`（危险色） | 至少一条 Error；点击 → 聚焦 Info 转发区 |

### 生命周期

```text
连接成功 → 启动 Profile 中 enabled 规则
         → Info / 状态栏更新 Listening | Error
关 Tab / 断线 → 立刻释放本机端口与桥接
Reconnect（MVP）→ 按 Profile 规则再拉；Temporary 可丢弃并提示
```

### 错误文案（交互关键）

| 情况 | UI |
|------|-----|
| 本机端口占用 | `15432 already in use` + Retry |
| 服务器拒绝转发 | 见下节「侦测 AllowTcpForwarding」；文案须区分「服务器禁止转发」与「本机端口占用 / 目标不可达」 |
| 目标不可达 | 规则可仍 Listening；首连失败时行下提示或标黄 |
| SSH Connecting | 转发区等待态，不报假 Error |

### 明确不做的交互

- 独立「Tunnel」Profile 类型当主路径。  
- 以终端命令为主管理入口（高级可后做）。  
- MVP 实时吞吐仪表。  
- 默认 `0.0.0.0` 监听。  
- 首页一级「转发」导航。

### MVP 交互范围

1. Profile：Local CRUD + enabled。  
2. 连接成功自动 Listen。  
3. Info 列表 + 单条启停 + 错误。  
4. 状态栏 `⇄ N`（推荐，成本低）。  
5. Temporary（可选，加分）。  

Remote / SOCKS：同一列表加 Type，表单随类型切换；Info 信息架构不变。

---

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

Info 列出本 pane 活跃转发（端口 + 状态）；详见上文「UI / 交互设计」。

## 服务端要求

| 模式 | Loom 之外 |
|------|-----------|
| Local / SOCKS | 通常只需远端 `sshd` 允许 TCP forwarding；**无需**装 Loom 或额外命令 |
| Remote | 同上，且更常被 `GatewayPorts`、防火墙限制；属运维配置，非 Loom 后端 |

禁用 `AllowTcpForwarding` 时客户端无法单方面绕过——如实报错即可。

### 侦测 `AllowTcpForwarding`（实现必做）

运维在服务器上可自查：

```bash
sudo sshd -T | grep -i allowtcpforwarding
# allowtcpforwarding yes  → 允许
# allowtcpforwarding no   → 禁止
# 无输出时 OpenSSH 默认多为 yes
```

**Loom 客户端不能假设能远程执行上述命令**（无 sudo、无 shell、或策略禁止）。实现时采用：

| 策略 | 做法 | 说明 |
|------|------|------|
| **主路径：尝试 + 归因** | 启用规则时真正开 `direct-tcpip`（或 Remote 的 `tcpip-forward`）；若对端返回 administratively prohibited / forwarding disabled 类错误 → 标记会话或规则为 **ForwardingDenied** | OpenSSH 无稳定的「查询开关」RPC；业界 GUI 客户端也是试出来的 |
| **缓存** | 同一 SSH 连接上首次判定后缓存；同 pane 内其它规则直接提示，避免每条都打含糊错 | 断线 / Reconnect 清空缓存 |
| **可选轻量探针（后期）** | 连上后对 `127.0.0.1` 某封闭端口做一次短生命周期 `direct-tcpip`，区分「禁止转发」vs「目标拒绝连接」 | MVP 可不做；若做须立刻关 channel，不占长期资源 |
| **禁止** | 为侦测而在远端跑 `sshd -T` / 改配置 / 要求用户粘贴配置 | 超出客户端职责 |

**UI 要求：**

| 状态 | 表现 |
|------|------|
| 已确认允许（至少一条成功 Listening 或探针成功） | 正常列表；不必常驻「已允许」徽章 |
| 已确认禁止 | Info / 规则区横幅或统一 Error：`Server disabled TCP forwarding (AllowTcpForwarding)`；该会话上启停转发给出相同说明；可链到文档一句「请管理员执行 `sshd -T \| grep allowtcpforwarding`」 |
| 尚未尝试 | 不预判为禁止；连接中不闪红 |

**归因优先级（勿误报）：**

1. 本机 bind 失败（端口占用）→ 本地错误，**不是** forwarding denied。  
2. Channel open 被服务器 administratively 拒绝 → **ForwardingDenied**。  
3. Channel 已开但连不上 target（connection refused）→ 目标/防火墙问题，转发权限通常仍可用。

## 非目标（当前）

- 自研非 SSH 的穿透协议（ngrok / frp 类）  
- 单独的「只转发、无 shell」连接类型（可日后评估）  
- Unix domain socket 转发（russh 有能力，不进 MVP）  
- 把转发做成产品核心卖点或首页一级导航  

## 分阶段

| 阶段 | 内容 | 状态 |
|------|------|------|
| 0 | 本规格文档 | **完成** |
| 1 | Local：会话级启停 + Profile 自动启用；默认 bind `127.0.0.1`；**含转发权限侦测与归因** | 未做 |
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
| UI | Profile 编辑 + Info 会话态 + 状态栏；见上文「UI / 交互设计」 |
| Teardown | 与 `tab_manager` 关 pane 路径对齐 SFTP 回收 |

## 验收（阶段 1）

1. SSH 连上后，Profile 中启用的 Local 规则在本机指定端口 Listening。  
2. 本机客户端连该端口，可访问配置的远端目标（如数据库）。  
3. 关 Tab 后端口释放，无残留 listener。  
4. 端口占用或服务器拒绝时有明确错误，UI 不卡死。  
5. 服务器 `AllowTcpForwarding no`（或等价拒绝）时，UI 明确归因 **ForwardingDenied**，不与「本机端口占用」「MySQL 未启动」混淆。  
6. 与同时进行的 SFTP 浏览/传输可共存（压力下可降级报错，但不拖死 shell）。

## 如何开做

用户明确说「做端口转发 / 实现 PORT_FORWARD / 做 Local 转发」等之后，再按阶段 1 切片；不要因讨论或「继续 roadmap」自动开工。
