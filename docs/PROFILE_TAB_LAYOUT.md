# Profile Tab 布局保存（layout + panes）

相关：[SESSION_PROFILE_IA.md](./SESSION_PROFILE_IA.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)。

> **状态**：已实现（显式 Tab Save / 打开恢复；关应用不自动写）。  
> **文档约定**：中文。

## 一句话目标

在 **不改变日常打开 / Split / 临时 Tab 体验** 的前提下，允许用户 **显式 Save** 当前 Tab 的分屏结构与各 Pane 工作目录；下次从侧栏打开该 Profile 时按快照恢复。关应用 **不** 自动写回。

## 产品定案

| 项 | 约定 |
|----|------|
| 保存时机 | **仅显式** Tab **Save** / **Save As…**；关 Tab / 关应用不写 |
| 侧栏 | **不加** open / dirty 指示点 |
| 日常行为 | 与现网一致；增量仅为「能保存并恢复 Tab 内多 Pane」 |
| 同一 Profile 多 Tab | 允许；**后 Save 的覆盖** Profile 上唯一一份快照 |
| 分栏比例 | **不持久化**；恢复时每个 Split **平分**（ratio = 0.5） |

## 数据模型

概念上（实现里 `connection` 仍用现有字段名 `kind`）：

```text
Profile {
  id, name,
  kind / connection: Local | Ssh | Wsl | Docker,  // 怎么连 + 公共默认 cwd
  forwards,                                       // SSH 等，与布局无关

  layout: Option<SavedPaneLayout>,   // 与 panes 成对
  panes:  Option<Vec<PaneSnapshot>>,
}
```

### 不变量

- `layout` 与 `panes` **同有或同无**。
- 都为 `None`：打开 = 现在行为（单 Pane，用公共 cwd）。
- 都为 `Some`：叶子数必须等于 `panes.len()`。
- **单 Pane 快照不落盘**：Save 时若只有 1 个 Pane，清空 `layout`/`panes`，只更新公共 cwd。

### 公共 cwd

| 类型 | 字段 |
|------|------|
| Local / WSL / Docker-local | `kind.Local.cwd`（Docker 容器路径见 panes；公共 cwd 语义弱化） |
| SSH / Docker-over-SSH | `kind.Ssh.cwd: Option<String>`（远端/容器路径字符串） |

- 每个 Profile **都有**公共默认 cwd 语义（可空）。
- 每个 Pane 另有 `PaneSnapshot.cwd`（可空则回退公共 cwd）。

### `PaneSnapshot`

```text
PaneSnapshot {
  cwd: Option<String>,
}
```

不存密码、滚动历史、未提交输入。

### `SavedPaneLayout`

只存结构与方向，**不存 ratio**：

```text
SavedPaneLayout =
  Leaf { pane: u32 }           // panes 下标
  | Split {
      axis: Horizontal | Vertical,
      first: SavedPaneLayout,
      second: SavedPaneLayout,
    }
```

读回运行时 `PaneLayout` 时，每个 Split 的 `ratio` 固定为 `0.5`。

## Save / Save As 写什么

| | 单 Pane | 多 Pane |
|--|---------|---------|
| Name | Save As 写入；Save 一般不改 | 同左 |
| 连接配置 | Save As 从会话拷贝；日常 Save 少动 | 同左 |
| **公共 cwd** | ✅ 可改并写入 | Edit Profile 可改；Tab Save **默认不改** 公共 cwd |
| layout + panes | 不写（保持 None） | ✅ 覆盖写入 |
| 密码明文 | ❌ | ❌ |

**Save As…**：新建 Profile；若当前 Tab 多 Pane，快照一并拷入；单 Pane 则只带公共 cwd。

## 打开 Profile

```text
若 layout/panes 为空 → 单 Pane，cwd = 公共 cwd
若有快照 → 建 Tab，按 SavedPaneLayout 挂 N 个叶子，
            各 Pane 用 panes[i].cwd ?? 公共 cwd 启动
```

## 设置对话框（Edit Profile）

- **可编辑**：Name、连接字段、**公共 cwd**。
- **只读**：`Panes: N`（无快照为 `1`；有快照为 `panes.len()`）。
- **可选**：`Clear saved tab` → `layout`/`panes` = None（公共 cwd 保留）。
- **不在对话框里**编辑分屏或各 Pane cwd。
- 无快照时可不渲染 Saved tab 区（对话框接近现状）。

新建 Profile：Pane 数显示为 **1**。

## 与 SESSION_PROFILE_IA 的关系

- Bound / Ephemeral、Split 默认 Ephemeral 等 **暂保持现网**（本 feature 不强制 Split Bound 化）。
- 扩展 Tab **Save**：从「仅 Local 写 start dir」升级为「单 Pane 写公共 cwd；多 Pane 写 layout+panes」。
- Docker Bound：允许 Save 快照（容器内路径）；单 Pane 时公共 cwd 按类型处理。

## 非目标（本 feature）

- 关应用自动 Save  
- 侧栏 dirty / 已打开圆点  
- 持久化拖拽比例  
- 恢复滚动缓冲 / 全局字体  

## 实现映射

| 块 | 位置 |
|----|------|
| 规格 | 本文 |
| 模型 | `src/model/profile.rs`（`PaneSnapshot` / `SavedPaneLayout`） |
| 运行时树 | `src/ui/pane_layout.rs`（to/from saved，ratio=0.5） |
| Save / 打开 | `src/ui/tab_manager.rs`、`src/ui/workspace_view.rs`、`workspace_store.rs` |
| 表单 | `local_form.rs` / `ssh_form.rs`（Pane 数 + Clear） |

## 验收

- [x] 单 Pane Tab Save → 只更新公共 cwd；`layout`/`panes` 仍为空；再打开单窗且目录正确  
- [x] 左右/上下分屏后 Save → 再打开结构正确且各 50%；各 Pane cwd 正确  
- [x] 嵌套分屏 Save → 树形正确，每层平分  
- [x] Save As 多 Pane → 新 Profile 带快照；原 Profile 不变  
- [x] Edit 显示正确 Pane 数；Clear 后变回单 Pane 打开  
- [x] 关应用不改变未 Save 的分屏；旧 workspace.json 可加载  
