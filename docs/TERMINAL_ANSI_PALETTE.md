# 终端 ANSI 配色预设（客户端，不碰服务器）

相关文档：[THEME.md](./THEME.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)、[BACKLOG.md](./BACKLOG.md)。

> **状态**：第一期 **已实现**。Settings → Terminal → ANSI colors（Default / Readable / High contrast）；默认 `default` 与历史配色一致。  
> **目标**：解决 SSH/Linux 上 `ls` 等使用标准 ANSI 蓝时、黑底对比度差的问题；**不改远端 shell / `LS_COLORS`**。

---

## 一句话

Settings 增加 **ANSI palette** 预设；Loom 只改「色号 → 屏幕颜色」的映射。服务器仍发同样的 ANSI，客户端可选更易读的蓝。

---

## 背景

| 现象 | 原因 |
|------|------|
| 同一 Loom 窗口，Local 与 SSH 颜色观感不同 | 两边是不同 shell（PowerShell vs bash）与不同着色逻辑 |
| SSH 上目录名偏暗蓝、难读 | GNU `ls` + `LS_COLORS` 常用 ANSI Blue；黑底对比差 |
| 不便 / 无权限改服务器 | 不能依赖写远端 `.bashrc` 或 `LS_COLORS` |

Loom Local 与 SSH **共用**同一 `ColorPalette`（见 `tab_manager::terminal_config`）。问题不在「SSH 专用渲染」，而在 **ANSI 蓝在当前调色板上偏暗**。

---

## 非目标（第一期不做）

- 改远端环境、注入 `LS_COLORS`、Profile startup 命令  
- Per-tab / per-profile 编码或配色（全局 Settings 即可）  
- 16 色手工色板编辑器、用户 JSON 主题包（交给 [THEME.md](./THEME.md) / A1）  
- 重解 scrollback、按程序猜色  

---

## 第一期产品模型

### Settings UI

放在现有 Terminal 设置区（Font size / Line numbers 旁）：

```
Terminal
  Font size       …
  Line numbers    …
  ANSI colors     [ Default ▾ ]    // Readable / High contrast
```

提示文案（短）：

> How ANSI colors are drawn in the terminal. Does not change the remote shell.

### 预设

| ID | 设置值 | 行为 |
|----|--------|------|
| **Default** | `default` | **与今天完全一致**（现有 `terminal_config` 色值）。**出厂默认；用户不改设置则零观感变化。** |
| **Readable** | `readable` | 调亮 `blue` / `bright_blue`（可选略提亮 `cyan`）；专治黑底目录蓝 |
| **High contrast** | `high_contrast` | 16 色整体抬对比（可选；第一期可先做 Default + Readable，HC 同发或紧随） |

### 持久化

`SettingsFile` 增字段（示意）：

```text
ansi_palette: "default" | "readable" | "high_contrast"
```

- `#[serde(default)]` → **`default`**  
- 缺字段 / 旧配置文件 → 行为与现在相同  
- **默认不做任何设置**：新装与未改过的用户保持现状  

### 作用范围

- **全局**：所有 Local / SSH pane 使用同一预设  
- 新建 tab：按当前 Settings 构建 `ColorPalette`  
- 设置变更：对已打开 pane 调用现有 `TerminalView::update_config` 热更新（与改字号同类）

---

## 实现要点（待点名实施时）

```
SettingsFile.ansi_palette
        │
        ▼
terminal_config(..., palette_id)
        │
        ▼
ColorPalette::builder()   // default | readable | high_contrast 常量表
        │
        ▼
TerminalConfig.colors → paint resolve()
```

| 模块 | 改动 |
|------|------|
| `model` `SettingsFile` | 增 `ansi_palette`，默认 `default` |
| `ui/settings.rs` | 下拉 / 分段控件 |
| `ui/tab_manager.rs` `terminal_config` | 按 id 选色；Default 分支保持现有 RGB 字面量不动 |
| 打开 / 重连 / 设置保存路径 | 传入 palette；变更时刷新已有 terminal |

**Readable 建议至少改：**

- ANSI 4 `blue`（`ls` 目录常用）  
- ANSI 12 `bright_blue`（`di=01;34` 粗体蓝常见映射）  

勿大改 red/green，避免语义色发飘。

---

## 与 Theme（A1）的关系

| 阶段 | 关系 |
|------|------|
| 第一期（本文） | 独立小开关，解决对比度；不阻塞 A1 |
| Theme 落地后 | 可将三套预设迁为内置 `terminal` 包；或 Settings 改为选 `terminal_theme_id`，本字段 deprecate / 别名 |

第一期 **不要** 做成半套主题系统。

---

## 第二期及以后（仅记一笔，不实施）

- 自定义 16 色 / JSON  
- Per-profile 覆盖  
- 与 UI Light/Dark 联动的「跟随主题」终端包  

---

## 验收（第一期）

- [x] 默认 / 未写配置：颜色与改前一致  
- [x] 选 Readable：SSH 上 `ls` 目录更易读（客户端调亮蓝）；无需改服务器  
- [x] Local / SSH 共用同一预设；设置变更热更新已打开 tab  
- [x] 重启后选项仍在  
- [x] 文案标明不修改远端 shell  

---

## 明确不做的捷径

- 用「SSH 专用 hack」绕过调色板（问题在 ANSI 映射，Local/SSH 应同一路径）  
- 默认改成 Readable（违反「默认不做任何设置」——会改变全员观感）  
- 第一期就做色板编辑器
