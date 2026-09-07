# Session / Profile / Group IA

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)。

> **状态**：实施中。  
> **文档约定**：新增规格默认中文。

## 概念（Unix 根目录隐喻）

侧栏工作区像一个文件系统根 **`/`**：

| 概念 | 类比 | 含义 |
|------|------|------|
| **Workspace 根** | `/` | 可直接挂 Profile 与 Group，无强制「先建文件夹」 |
| **Group** | 目录 | 可嵌套；其下可再挂 Profile 与子 Group |
| **Profile** | 文件 | 可持久连接配置（Local / SSH）；可在根或任意 Group 下 |
| **Tab** | 打开的窗口页 | 中间工作区一页：一棵 split 布局 + 若干 Pane |
| **Pane** | 页内一块终端 | **真正的会话 IO 单位**（一条 Local/SSH） |

口诀：Group 是目录，Profile 是文件，Tab 是页签，Pane 是活会话。

```text
Profile（侧栏收藏）
    │  双击打开
    ▼
Tab ── layout 树 ── Pane（叶子）
                 └── Pane
```

## 数据形状

```text
WorkspaceFile {
  profiles: Vec<Profile>,   // 根级「文件」
  groups: Vec<Group>,       // 根级「目录」
}
Group {
  profiles: Vec<Profile>,
  children: Vec<Group>,     // 嵌套目录
  collapsed: bool,
}

PaneSession {
  profile_id: Option<Uuid>,      // Some = Bound；None = Ephemeral
  auth_profile_id: Option<Uuid>, // Ephemeral SSH 继承的凭据 Profile（≠ Bound）
  session_password: Option<String>, // 仅内存；Split/Duplicate 从源 pane clone；不落盘
  kind, label, state, terminal, …
}
```

旧版仅有 `groups[].profiles`、无根级 `profiles` / `children`：加载时兼容（缺省字段为空）。

## Bound vs Ephemeral

| | Bound | Ephemeral |
|--|-------|-----------|
| `profile_id` | `Some(id)` | `None` |
| 侧栏 | 对应 Profile | 无 |
| 重启 | 可进 `open_tabs` | 不恢复 |
| 凭据 Profile | `profile_id` | `auth_profile_id`（从源会话继承） |
| 会话密码 | 打开/弹窗/keyring 成功后写入 `session_password` | Split/Duplicate 从源 **clone** |

**SSH 密码解析优先级**（`resolve_ssh_auth` / 调用方合并后传入）：

1. 显式 override（PasswordPrompt / `*_with_password`）  
2. **`pane.session_password`（内存）**  
3. OS keyring（`credentials_profile_id()`）  
4. 弹 PasswordPrompt  

**凭据 Profile id** 仍用 `credentials_profile_id() = profile_id.or(auth_profile_id)`（keyring 键、转发、弹窗绑哪个 Profile）。  
**不要**用 `auth_profile_id` 判断是否 Bound。  
**`session_password` 不持久化**；进程退出即丢。未勾选 Remember 时，同进程内 Split/Duplicate/Reconnect 仍可用内存密码。

**Bound Tab** 定义：该 Tab 内 **任意** Pane 的 `profile_id` 为 `Some`（不要求 focused 是 Bound）。  
持久化 `snapshot_for_persist` 必须用此定义，否则 Split 后焦点在 Ephemeral 叶子会丢掉恢复。

## 硬规则

1. **Profile 可不属于任何 Group**（在根 `workspace.profiles`）。
2. **Group 可嵌套**（`children`）。
3. **选中 Group 时 New\*** → 创建物挂到**该 Group**；选中根 / 根级 Profile → New\* 进**根**；选中 Group 内 Profile → New\* 进**父 Group**。
4. **新建 / 侧栏 Duplicate Profile → 只改收藏，不自动打开** Session。
5. **工作区再开为临时**：Ctrl+T、Tab/终端 Duplicate、Split → **Ephemeral**（不进侧栏）。文案均叫 **Duplicate**，靠 context 区分。
6. **点侧栏 Profile → Bound Session**（可持久恢复）。
7. **Tab 右键「Save to…」**（仅当 **focused Pane 为 Ephemeral**）：新建 Profile，并把该 Tab 内 **所有 Ephemeral Pane** bind 为 Bound；已有 Bound 叶子不动。
8. **重启不恢复临时 tab**；`open_tabs` 只写 Bound Tab。
9. **Bound Local**：关闭/持久化时把 Bound Pane 的 shell cwd 写回 Profile。
10. **SSH 密码 UI**（打开 / Duplicate / Split / Reconnect）与底层 resolve 同一套优先级；仅当内存与 keyring 都没有时才弹窗，禁止只改 status / `eprintln`。
11. **Reconnect 粒度**：状态栏 = 该 Tab **全部** Pane；Failed 占位按钮 = **单** Pane；两者密码路径对称（优先各 pane 的 `session_password`）。
12. **Connecting ≠ Failed**：`terminal == None` 时两者都有；Close/Reconnect 按钮只给 Failed/Disconnected。
13. **Split / Duplicate** 必须从源 pane **clone `session_password` 与 `auth_profile_id`**，使同 Tab 分屏无需再问已认证过的密码。
14. **WSL**：侧栏 New WSL → 扫描本机 distro → 建成带 `args: ["-d", name]` 的 Local Profile。打开/重连在后台线程 preflight（`wsl --list` + 短探针 `wsl -d … -- true`）；发行版已卸载或 **vhdx 丢失**（仍出现在列表里）时该 Pane 进 Failed，**不得**拖垮整个应用。有 `args` 时禁止 shell 回退到默认 pwsh/cmd。

## 操作对照

| 操作 | 新建 Profile？ | 新建 Tab？ | 新建 Pane？ | 新会话类型 |
|------|----------------|------------|-------------|------------|
| 双击侧栏 Profile | 否 | **是** | 是（1） | Bound |
| 侧栏 Duplicate | **是**（复制配置） | 否 | 否 | — |
| Tab / 终端 / Ctrl+Shift+D Duplicate | 否 | **是** | 是（1） | Ephemeral（SSH 带 `auth_profile_id`） |
| Split | 否 | 否 | **是** | Ephemeral（SSH 带 `auth_profile_id`） |
| Ctrl+T | 否 | **是** | 是（1） | Ephemeral Local |
| Save to… | **是** | 否 | 否 | focused 及同 Tab 其它 Ephemeral → Bound |

## Session 来源（Pane 级）

| 来源 | `profile_id` | `auth_profile_id` | `session_password` | 重启 |
|------|--------------|-------------------|-------------------|------|
| 侧栏打开 | `Some` | `None` | 认证成功后写入 | 可恢复（密码不恢复） |
| Ctrl+T | `None` | `None` | — | 否 |
| Duplicate Tab / Split | `None` | 继承 credentials | 从源 clone | 否 |
| Save to… | 变为 `Some` | 清掉 | 保留（仍仅内存） | 之后可恢复 |

## Duplicate（同文案）

| Context | 行为 |
|---------|------|
| 侧栏 Profile | 复制到**同一父节点**；不打开 |
| Tab 栏 / 终端菜单 / Ctrl+Shift+D | 再开 **临时** Tab（克隆 focused Pane，含 `session_password`） |

## 持久化

- `workspace.json`：根 `profiles` + 嵌套 `groups`。  
- `ui_state.open_tabs`：仅 Bound Tab（`bound_profile_id()`）。  
- **永不**把 `session_password` 写入磁盘。  
- 启动：只按 Bound 重开（单 Pane；split 树暂不持久）；重启后需 keyring 或再弹窗。

## 实现锚点

- `PaneSession` / `session_password` / `SessionOpResult`：`src/ui/tab_manager.rs`
- 密码后续动作：`PendingPasswordAction` in `src/ui/workspace_view.rs`
- 单 Pane 重连事件：`AppBusEvent::ReconnectPane`

## 非目标（本阶段）

- 完整 DnD 把 Group 拖成另一 Group 的子节点（可后续）。  
- 自动「常用升级」。  
- 删目录时对开着 Bound tab 的复杂级联 UI。  
- 持久化 split 树。
