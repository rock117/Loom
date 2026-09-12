# Docker 会话（exec + Files / docker cp）

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[CONTEXT_PANEL.md](./CONTEXT_PANEL.md)、[SFTP_POOL.md](./SFTP_POOL.md)、[SESSION_PROFILE_IA.md](./SESSION_PROFILE_IA.md)、[BACKLOG.md](./BACKLOG.md)。

> **状态**：阶段 1（本机列表 + exec）**已实现**；阶段 2（本地 Files + `docker cp`）**已实现**；阶段 3（SSH Profile 列表 + exec + Files/`docker cp`）**已实现**（选择器远端 `docker ps` + Save Docker-over-SSH Profile）。阶段 4 仍须用户明确点名。  
> **文档约定**：中文。

## 一句话目标

把 **进入 Docker 容器** 做成与 **SSH 进服务器** 同级的会话体验：中间是交互式 shell，右侧 Context **Files** 可浏览与传文件——传输走 **`docker cp`**，而不是 SFTP。支持 **本机 Docker** 与 **经已有 SSH Profile 的远端 Docker**。

## 背景

高频路径不是「在 Loom 里做 Docker Desktop」，而是：

1. 找到 running 容器（本机或某台 SSH 宿主机上）  
2. `docker exec -it …` 进 shell  
3. 偶发把文件拷进 / 拷出容器  

手敲 exec / cp 完全可用，但重复成本高。Loom 已有 Local / SSH Profile + Context Files；Docker 应复用同一套会话/Tab 模型，只换后端。

## 产品模型

| 维度 | Local | SSH | Docker（本规格） |
|------|-------|-----|------------------|
| 入口 | 本机 shell Profile | `user@host` Profile | **工具栏 Docker 图标 → 选宿主 → 选容器**（见下） |
| Shell | portable-pty | russh PTY | **`docker exec -it`**（本机 CLI 或经 SSH）→ 同一套终端 pane |
| Files | 本机目录（已有） | SFTP | **浏览 + `docker cp` 传输** |
| 状态栏 cwd | 本机 / OSC | OSC | **仅容器内路径（OSC）**；不显示 `docker.exe` 宿主 cwd |
| Transfers 页脚 | — | 已有 | **复用**（Queued / 进度 / 取消） |
| Info | Local | SSH 摘要 | 容器名、ID、镜像、状态、宿主（本机 / 哪个 SSH Profile） |

心智：

- **容器 = 一种会话目标**，不是侧栏里的「Docker 管理器」。  
- **侧栏仍是 Profile 收藏**（Local / SSH / WSL）；**不要**再挂一棵常驻「Docker 树」（与 Profile 语义重合、容器又是瞬时资源）。  
- SSH Profile = 宿主机；进远端容器 = 在该主机上跑 `docker`。

---

## 入口 UX（已定）

点击侧栏 / 工具栏 **Docker 图标**，弹出选择器（同一对话框，两步或同页分区均可）：

```text
① 模式（宿主）
   ┌─────────────┬──────────────────┐
   │  本地        │  SSH Profile     │
   └─────────────┴──────────────────┘
         │                  │
         ▼                  ▼
② 容器列表              先选一个 SSH Profile
   （本机 docker ps）         │
                              ▼
                         该主机上的 docker ps 列表

③ 点击某一 running 容器 → 新 Tab：交互式 exec shell
```

### 行为细则

| 项 | 约定 |
|----|------|
| 模式 1 · 本地 | 本机 `docker ps`（优先 running）；刷新 / 搜索过滤 |
| 模式 2 · SSH Profile | 列出工作区里已有的 **SSH Profile**；选中后再拉该机容器列表（经 SSH 执行 `docker ps`） |
| **Name** | 与 WSL 相同：可编辑 Profile 名；选容器时若未手改则默认填容器名（重名自动加后缀） |
| 按钮 | **Save**（只写入侧栏）/ **Save & Open**（写入并打开 Tab）；Enter = Save & Open |
| 无 SSH Profile | 模式 2 显示空态 + 引导去 New SSH |
| 打开结果 | Bound Session（侧栏可见 Docker 图标 Profile）；`docker exec -it …` |
| 异步 | `docker ps` / SSH 列举均在后台；**禁止堵 UI 线程**；Windows 子进程用 `platform::new_command`（`CREATE_NO_WINDOW`） |
| 错误 | 本机无 Docker、引擎未起、远端无 `docker`、权限不足 → 可操作错误文案，不卡死 |

实现阶段上：UI 骨架可先同时露出两种模式；**本地列表+exec 先可用**，SSH 模式可先灰显或进阶段 3 再接通后端。

---

## 用户流程（进容器之后）

```
选宿主 + 选容器 → 新 Tab（交互 shell，体验对齐 SSH Tab）
        ↓ 打开 Context → Files
浏览容器内文件系统；上传 / 下载走 docker cp；Transfers 显示进度
```

## Shell：`docker exec`

### 行为

- 对 **running** 容器执行交互式 exec，分配伪终端（`-i -t`）。  
- 默认命令：优先 `bash`，不存在则 `sh`（可后续覆盖）。  
- **本地宿主**：本机 spawn `docker exec -it …` → 现有 PTY 管线。  
- **SSH 宿主**：经该 Profile 的 SSH 会话在远端执行 `docker exec -it …`（实现可选：复用/另开控制通道，但须不堵 UI）。  
- 成功后：输入 / 滚动 / 分屏 / 右键 / 查找与现有终端 **同一条 UI 路径**。  
- 容器停止或 exec 退出：Tab 表现对齐 SSH 断开（提示；重连策略后期再定）。

### MVP 范围

| 做 | 暂不做 |
|----|--------|
| 入口：Docker 图标 → 本地 / SSH Profile → 容器列表 | 侧栏常驻 Docker 树 |
| 本机 + 经 SSH 的 exec（分阶段接通） | 完整镜像 / 网络 / volume / Compose UI |
| 只 prioritise **running** | attach 复用已有会话的复杂逻辑 |
| | Windows 容器特殊路径（先 Unix 容器） |
| | 自定义 docker host / `docker context`（可后期） |

### 延后增强

- exec 选项：`-u`、工作目录、entrypoint 覆盖。  
- 钉选「SSH Profile + 容器选择器」为侧栏 Profile（名会变，标签选择器更稳）。

## Files：浏览 + `docker cp`

与 [CONTEXT_PANEL.md](./CONTEXT_PANEL.md) 同一套 Explorer UX；后端从 SFTP 换成 Docker 文件桥。

### 浏览（列目录 / 进入 / 上级 / Home）

- **不**解析用户在 shell 里敲的 `ls` 作为唯一真相。  
- MVP：对容器再开 **非交互** `docker exec` 做结构化列举；失败时 Files 显示明确错误。  
- 路径栏显示容器内绝对路径；Home 用 `/` 或探测到的 `$HOME`。  
- **浏览与传输分车道**（对齐 [SFTP_POOL.md](./SFTP_POOL.md)）：大文件 cp 进行中仍应能切目录；取消须结束对应进程。

### 传输：`docker cp`

| 宿主 | 下载 / 上传 |
|------|-------------|
| 本地 | 本机 `docker cp <container>:<path> <local>` 与反向 |
| SSH | 远端执行 `docker cp`，经 SSH/SFTP 或管道落到本机（实现时选定一种稳定方案；须可取消） |

- Transfers：**按 pane 隔离**；文案英文（Queued / Scanning / Done / Failed 等）。  
- **× / Clear**：取消 in-flight 进程，不是只删 UI 行。  
- 进度：尽力而为（`docker cp` 原生进度有限时可先近似）。

### 与 SFTP 的差异

- 权限、属主、特殊文件：以 Docker / `docker cp` 行为为准。  
- 不把容器 Files 伪装成「又一条 SFTP」；协议不同，只复用 **UI + Transfers 模型**。

## 架构草图（实现时）

```
Docker 选择器（图标）
├─ Mode::Local     → 本机 docker ps / exec / cp
└─ Mode::Ssh(profile_id) → SSH 上 docker ps / exec / cp

Docker pane
├─ Shell：exec -it → 现有 terminal / PTY（或 SSH 远端 PTY）管线
└─ Files 桥（对标 SftpPool）
   ├─ Lane::Browse
   └─ Lane::Transfer
```

- UI 仍只绑 **焦点 pane** 的 Context。  
- 关 Tab：立刻取消该 pane 的 cp / 浏览 / 列举子进程，不留僵尸。

## Profile / 侧栏 IA

| 做 | 不做 |
|----|------|
| 侧栏放 Local / SSH / WSL / **Docker**（`docker exec` Local Profile） | **不做**侧栏常驻 Docker 容器树（瞬时 `docker ps`） |
| Docker 入口 = 图标弹层（模式 + **Name** + 列表 + Save / Save & Open） | 把瞬时列表行直接当 Profile（须经 Name 确认写入） |
| 后期可选：SSH 宿主 + 容器选择器钉选 | 第三套与 Group/Profile 平行的导航 |

## Info 面板

只读摘要：类型 Docker、宿主（本机 / SSH）、容器名、短 ID、镜像、状态；以及 **Ports**（宿主→容器发布端口）与 **Volumes**（bind / volume / tmpfs 挂载）。不提供完整 Docker 管理 UI。

## 非目标（当前规格）

- Docker Desktop 式全功能管理  
- 在 Files 里做完整远程编辑器  
- 用 shell 输出冒充文件树的主路径  
- K8s `kubectl exec`（单独规格）  
- 与 SSH SFTP 混用同一 worker（仅复用 UI / Transfers）

## 分阶段

| 阶段 | 内容 | 状态 |
|------|------|------|
| 0 | 本规格（含图标双模式入口） | **完成** |
| 1 | 入口 UI + **本地** 容器列表 + exec 进 shell（无 Files） | **已实现**（本机） |
| 2 | **本地** Files 浏览 + `docker cp` + Transfers / 取消 | **已实现** |
| 3 | 模式 **SSH Profile**：远端列表 + exec + cp | **已实现**（picker SSH 列表 / Save；exec + Files 走 Docker-over-SSH Profile） |
| 4 | 钉选 Profile、exec 选项、进度精细化 | 未做 |

阶段 3：Docker 选择器 SSH 模式选 Profile → 后台 `list_running_containers_ssh`（缺密码走 `NeedSshPassword`）→ Save 为 `new_ssh_docker_profile`。

## 实现映射（落地时填写）

| 块 | 预期位置 |
|----|----------|
| 规格 | `docs/DOCKER_SESSION.md`（本文） |
| 选择器 UI | `src/ui/docker_picker.rs`；侧栏 `icons/ui/docker.svg` |
| 会话 / exec | `docker.rs` + Local exec；SSH：`ProfileKind::Ssh.docker_container` + PTY `request_exec` |
| Files 桥 | 本机 `docker_fs.rs`；远端 `docker_ssh.rs`（exec 列举 + `docker cp` 暂存 + SFTP） |
| Context | `FilesKind::Docker`（Local 或 Docker-over-SSH；后者优先于宿主 SFTP） |
| Pane | Local docker argv，或 SSH + `docker_container` |

## 验收

**阶段 1：** 点 Docker 图标 → 选本地 → 见 running 列表 → 打开可交互 Tab；UI 不卡死。  

**阶段 1+2：** 本地 Files + `docker cp` + 取消；关 Tab 无僵尸进程。  

**阶段 3：** 点 Docker → 选 SSH Profile → 见该机容器列表 → exec / cp 可用；断线与无 docker 有明确错误。

## 如何开做

用户明确说「做 Docker 会话 / 实现 DOCKER_SESSION」等之后，再按阶段 1 → 2 → 3 切片提交；不要因「继续 roadmap」自动开工。
