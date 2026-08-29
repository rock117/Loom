# Docker 会话（exec + Files / docker cp）

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[CONTEXT_PANEL.md](./CONTEXT_PANEL.md)、[SFTP_POOL.md](./SFTP_POOL.md)、[BACKLOG.md](./BACKLOG.md)。

> **状态**：规格已定，**尚未实现**。实现须由用户明确点名后开始（与 backlog 纪律一致）。  
> **文档约定**：新增规格默认中文。

## 一句话目标

把 **进入 Docker 容器** 做成与 **SSH 进服务器** 同级的会话体验：中间是交互式 shell，右侧 Context **Files** 可浏览与传文件——传输走 **`docker cp`**，而不是 SFTP。

## 背景

高频路径不是「在 Loom 里做 Docker Desktop」，而是：

1. 找到 running 容器  
2. `docker exec -it …` 进 shell  
3. 偶发把文件拷进 / 拷出容器  

手敲 exec / cp 完全可用，但重复成本高。Loom 已有 Local / SSH Profile + Context Files；Docker 应复用同一套 IA，只换后端。

## 产品模型

| 维度 | Local | SSH | Docker（本规格） |
|------|-------|-----|------------------|
| 入口 | 本机 shell Profile | `user@host` Profile | 容器（名 / ID）或 Docker Profile |
| Shell | portable-pty | russh PTY | **`docker exec -it`** → 同一套终端 pane |
| Files | 空状态（暂） | SFTP | **浏览 + `docker cp` 传输** |
| Transfers 页脚 | — | 已有 | **复用**（Queued / 进度 / 取消） |
| Info | Local | SSH 摘要 | 容器名、ID、镜像、状态等 |

心智：**容器 = 一种会话目标**，不是侧栏里的「Docker 管理器」。

## 用户流程（目标体验）

```
侧栏：Docker 分组 / 刷新容器列表
        ↓ 点击 running 容器（或已保存的 Docker Profile）
新 Tab：交互式 shell（与 SSH Tab 无异）
        ↓ 打开 Context → Files
浏览容器内文件系统；上传 / 下载走 docker cp；Transfers 显示进度
```

## Shell：`docker exec`

### 行为

- 对 **running** 容器执行交互式 exec，分配伪终端（`-i -t`）。
- 默认命令：优先 `bash`，不存在则 `sh`（可后续在 Profile 里覆盖）。
- 成功后：输入 / 滚动 / 分屏 / 右键菜单 / 查找等与现有终端 **完全同一条 UI 路径**。
- 容器停止或 exec 退出：Tab 表现对齐 SSH 断开（可提示；重连策略后期再定）。

### MVP 范围

| 做 | 暂不做 |
|----|--------|
| 本机 Docker（`docker` CLI 在 PATH） | 完整镜像 / 网络 / volume 管理 UI |
| 只 prioritise **running** 容器 | 附着到已有 attach 会话的复杂复用 |
| 一键进 shell | Compose 编排面板、build UI |
| | Windows 容器特殊路径（先 Unix 容器） |

### 第二期（明确延后）

- **经 SSH 的远端 Docker**：先 SSH 到宿主机，再对该机执行 `docker exec` / `docker cp`（运维高频，但多一跳，单独切片）。
- 可选：`docker context`、自定义 docker host / sock。
- exec 选项：`-u`、工作目录、entrypoint 覆盖。

## Files：浏览 + `docker cp`

与 [CONTEXT_PANEL.md](./CONTEXT_PANEL.md) 同一套 Explorer UX；后端从 SFTP 换成 Docker 文件桥。

### 浏览（列目录 / 进入 / 上级 / Home）

- **不**解析用户在 shell 里敲的 `ls` 输出作为唯一真相（避免和 TTY 抢输出）。
- MVP 推荐：对容器再开 **非交互** `docker exec` 跑结构化列举（例如 `ls -la --quoting-style=…` 或小脚本），或等价稳定方式；失败时 Files 显示明确错误。
- 路径栏显示容器内绝对路径（如 `/var/log`）；Home 可用容器内 `/` 或探测到的 `$HOME`。
- **浏览与传输分车道**（对齐 [SFTP_POOL.md](./SFTP_POOL.md) 思想）：大文件 `docker cp` 进行中仍应能切目录；取消传输须能结束对应进程，释放 Transfer 车道。

### 传输：`docker cp`

| 方向 | 含义 |
|------|------|
| 下载 | `docker cp <container>:<remote> <local>` |
| 上传 | `docker cp <local> <container>:<remote>` |

- 单文件与目录树：MVP 与 SSH Files 对齐（至少支持选中文件/文件夹下载；上传可先单文件，再文件夹）。
- Transfers：**按 pane 隔离**；状态文案英文（Queued / Scanning / Done / Failed 等），与现网一致。
- **× / Clear**：取消 in-flight 的 `docker cp`（杀进程或等价），不是只删 UI 行。
- 进度：尽力而为（`docker cp` 原生进度有限时可先做字节/文件计数近似，再迭代）。

### 与 SFTP 的差异（需在 Info / 错误文案中可感知）

- 权限、属主、稀疏文件、特殊文件：`docker cp` ≠ SFTP，行为以 Docker 文档为准。
- 不另开「只为 Files 服务的第二条 SSH」；本机 Docker 只依赖本机 CLI。
- 远端 Docker（二期）才是 `ssh` + 远端 `docker cp`。

## 架构草图（实现时）

```
Docker pane
├─ Shell：spawn `docker exec -it <id> <shell>` → 现有 terminal / PTY 管线
└─ Files 桥（逻辑上对标 SftpPool）
   ├─ Lane::Browse   ← 列目录 / Home（短生命周期 exec）
   └─ Lane::Transfer ← docker cp（可取消）
```

- UI 仍只绑 **焦点 pane** 的 Context 面板。
- 关 Tab：立刻取消该 pane 的 cp / 浏览子进程，不留僵尸。
- 本机无 Docker / 权限不足 / 守护进程未起：连接或刷新列表时给出可操作错误（安装提示、引擎未运行等），**禁止拖死 UI 线程**。

## Profile / 侧栏 IA（建议）

MVP 两种入口可并存，实现时选一种为主：

1. **动态列表**：侧栏「Docker」树，刷新 `docker ps`，点击即开 Tab（少配置）。  
2. **可保存 Profile**：钉住常用容器名 / 组合过滤器（容器名会变，钉 **标签选择器** 更稳，后期再做）。

分组、搜索、颜色标记与 SSH Profile 同一侧栏体系，避免第三套导航。

## Info 面板

只读摘要示例：类型 Docker、容器名、短 ID、镜像、状态、本机/远端（二期）、当前 Files 路径、终端尺寸。

## 非目标（当前规格）

- Docker Desktop 式全功能管理（镜像构建、网络编辑、Compose 可视化编排）
- 在 Files 里做完整远程编辑器
- 用 shell 输出冒充文件树的主路径
- K8s `kubectl exec`（可类比，但单独规格，勿塞进本篇 MVP）
- 与 SSH SFTP 混用同一 worker（协议不同，仅复用 **UI** 与 Transfers 模型）

## 分阶段

| 阶段 | 内容 | 状态 |
|------|------|------|
| 0 | 本规格文档 | **完成** |
| 1 | 本机：容器列表 + exec 进 shell（无 Files） | 未做 |
| 2 | 本机：Files 浏览 + `docker cp` + Transfers / 取消 | 未做 |
| 3 | 经 SSH 的远端 Docker exec + cp | 未做 |
| 4 | Profile 钉选、exec 选项、进度精细化 | 未做 |

## 实现映射（落地时填写）

| 块 | 预期位置 |
|----|----------|
| 规格 | `docs/DOCKER_SESSION.md`（本文） |
| 会话 / exec | `src/session/`（新建 docker 模块，或与 local spawn 并列） |
| Files 桥 | 对标 `src/session/sftp.rs` 的 docker 文件 API |
| UI | 侧栏列表 + 现有 `context_panel.rs` 按会话类型切换后端 |
| Pane | `PaneSession` 增加 docker 句柄（exec 子进程 + files 桥） |

## 验收（阶段 1+2）

1. 本机有 running 容器时，可从 Loom 一键打开 Tab，shell 可交互，体验近似 SSH Tab。  
2. Context → Files 可浏览容器内目录；上传/下载经由 `docker cp` 完成。  
3. 传输中可取消；Browse 与 Transfer 互不长期堵死。  
4. 无 Docker / 容器已停时有明确错误，应用不卡死。  
5. 关 Tab 后无残留 `docker cp` / 列举进程。

## 如何开做

用户明确说「做 Docker 会话 / 实现 DOCKER_SESSION」等之后，再按阶段 1 → 2 切片提交；不要因「继续 roadmap」自动开工。
