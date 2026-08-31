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
| **Tab / Session** | 打开的进程/会话 | 中间工作区；Bound 或 Ephemeral |

口诀：Group 是目录，Profile 是文件，Tab 是正在跑的进程。

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
```

旧版仅有 `groups[].profiles`、无根级 `profiles` / `children`：加载时兼容（缺省字段为空）。

## 硬规则

1. **Profile 可不属于任何 Group**（在根 `workspace.profiles`）。
2. **Group 可嵌套**（`children`）。
3. **选中 Group 时 New\*** → 创建物挂到**该 Group**（New Group = 子目录；New Local/SSH = 该目录下的文件）。  
   **选中根级 Profile / None** → New Profile 进**根**；New Group 进**根**。  
   **选中某 Group 内的 Profile** → New* 进**该 Profile 的父 Group**。
4. **新建 / 侧栏 Duplicate Profile → 只改收藏，不自动打开** Session。
5. **工作区再开为临时**：Ctrl+T、Tab Duplicate、Split → **Ephemeral**（不进侧栏）。文案均叫 **Duplicate**，靠 context 区分。
6. **点侧栏 Profile → Bound Session**（可持久恢复）。
7. **Tab 右键「Save to…」** → 可选根或某 Group；升级为 Profile 并 Bound。
8. **重启不恢复临时 tab**；`open_tabs` 只写 Bound。

## Session 来源

| 来源 | `profile_id` | 重启 |
|------|--------------|------|
| 侧栏打开 | `Some`（Bound） | 可恢复 |
| Ctrl+T / Duplicate Tab / Split | `None`（Ephemeral） | 不恢复 |
| Save to… | 变为 `Some` | 之后可恢复 |

## Duplicate（同文案）

| Context | 行为 |
|---------|------|
| 侧栏 Profile | 复制到**同一父节点**（根或同 Group）；不打开 |
| Tab / Ctrl+Shift+D | 再开 **临时** tab |

## 持久化

- `workspace.json`：根 `profiles` + 嵌套 `groups`。  
- `ui_state.open_tabs`：仅 Bound。  
- 启动：只按 Bound 重开。

## 非目标（本阶段）

- 完整 DnD 把 Group 拖成另一 Group 的子节点（可后续）。  
- 自动「常用升级」。  
- 删目录时对开着 Bound tab 的复杂级联 UI。
