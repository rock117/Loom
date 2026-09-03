# 主题系统（Theme）实现方案

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[BACKLOG.md](./BACKLOG.md)（A1 / A2）、[ZED_THEME.md](./ZED_THEME.md)（Zed 机制对照，GPL 源码勿拷贝）。

> **状态**：规格已定，**尚未实现**。实现须由用户明确点名后开始（与 backlog 纪律一致）。  
> **文档约定**：新增规格默认中文。

## 一句话目标

把当前编译期写死的 `theme.rs` 常量，演进为 **可切换的运行时主题**：先内置 Dark / Light（及可选高对比），再支持用户 JSON；UI chrome 与终端配色分层、可联动。

## 背景与现状

| 现状 | 影响 |
|------|------|
| `src/shared/theme.rs` 全是 `pub const` 色值 / 间距 | 无法热切换；换主题必须改代码重编译 |
| UI 各处直接 `theme::PANEL_BG` 等 | 迁移成本在「调用点」，不在配色本身 |
| 终端 `ColorPalette` 与 UI 主题无关 | 换 UI 主题后终端仍可能「另一套风格」 |
| Settings 仅有 font / line numbers | 尚无 `theme` 字段；ANSI 预设见独立规格 [TERMINAL_ANSI_PALETTE.md](./TERMINAL_ANSI_PALETTE.md)（可先于完整 Theme 落地） |
| 窗口 `appears_transparent: false`（原生标题栏） | 系统标题栏颜色不受应用主题控制，易与底栏/内容割裂 |

业界（VS Code / Zed / Windows Terminal）普遍做法：

- **Client-Side Decorations**：自绘标题栏，与 StatusBar 同色阶  
- **Token 契约**：surfaces / text / accent，而不是散落魔法色  
- **UI 主题与终端 palette 分两层**，可「跟随」或「独立选择」

Zed 具体怎么加载 JSON、Global、缺省 merge、热刷新，见 [ZED_THEME.md](./ZED_THEME.md)。Loom **不复制** Zed 上百个 editor/syntax token；只采用同一套运行时机制。

## 非目标（本规格不覆盖）

- 任意 CSS / 完整皮肤引擎  
- 主题市场、远程下载主题包  
- 插件加载主题（留给 backlog E1；主题适合做插件，但须在核心稳定之后）  
- 把间距 / 圆角 / 字号全部主题化（第一期保持常量即可）

## 产品模型

```
Settings.theme_id  ──►  ActiveTheme（Global / Entity）
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
         UI tokens      终端 palette     标题栏 / 边框
      (sidebar/tabs/   (ANSI 16 +       (CSD，与 chrome
       status/…)        default fg/bg)    同 token)
```

| 层 | 内容 | 切换行为 |
|----|------|----------|
| **UI** | 表面色、文字、边框、accent、hover/selection | 立刻 `notify` 整窗重绘 |
| **Terminal** | 默认前景/背景、16 ANSI（及扩展若需要） | 默认可跟随 UI；允许设置里单独指定 |
| **Chrome 几何** | 标题栏高度、TabBar/StatusBar 高度 | 第一期仍用现有常量；仅颜色跟主题 |

### 内置包（第一期）

| ID | 说明 |
|----|------|
| `loom-dark` | 现有 `theme.rs` 数值原样迁入（行为不变） |
| `loom-light` | 浅色对照包（对比度对齐现有密度） |
| （可选）`loom-hc-dark` | 高对比（对齐 backlog A2，可与 A1 同发或紧随） |

### 用户主题（第二期）

- 路径示例：`~/.loom/themes/*.json`（或与现有 config 目录一致）  
- 只覆盖 **已知 token 键**（见下节）；缺字段回落到所选内置基座  
- 非法 JSON / 非法色值：Settings 提示，不崩溃，保持上一有效主题

---

## Loom 可修改的主题设置

对照 [ZED_THEME.md](./ZED_THEME.md)：Zed 有上百个 editor/syntax/vim token。Loom 只开放 **终端客户端壳 UI + 终端 palette** 所需的一小截。下列为契约；实现时 Settings / JSON 不得擅自增加未列键（可后续修订本文再扩）。

### A. Settings 里选什么（产品开关）

| 设置键 | 类型 | 默认 | 说明 |
|--------|------|------|------|
| `theme` | string | `"loom-dark"` | 当前 UI 主题 id（内置或用户文件 stem） |
| `appearance` | `"manual"` \| `"system"` | `"manual"` | `system` 时在 `loom-dark` / `loom-light`（或 HC）间跟随 OS；`manual` 只用 `theme` |
| `terminal_theme` | `null` \| string | `null` | `null` = 终端默认 fg/bg（及可选整盘 ANSI）**跟随** UI 主题；非空 = 独立主题 id / 文件 |
| `ansi_palette` | 已有枚举 | `"default"` | 见 [TERMINAL_ANSI_PALETTE.md](./TERMINAL_ANSI_PALETTE.md)。与 Theme **并存**：指定时覆盖主题里的 16 ANSI；未指定时可用主题自带 ANSI 或内置推导 |

字号 / 行号等仍走现有 Settings，**不属于主题包**。

### B. UI 色 token（主题包 / 用户 JSON 可改）

对应今天 `src/shared/theme.rs` 的色常量；JSON 键用 **snake_case**（与运行时字段一致）。

| JSON / 字段 | 现状常量 | 用在哪 |
|-------------|----------|--------|
| `bg` | `BG` | 主内容区底（终端所在列） |
| `sidebar_bg` | `SIDEBAR_BG` | 左侧 Profiles |
| `panel_bg` | `PANEL_BG` | Tab 栏、右栏、状态栏等面板面 |
| `elevated` | `ELEVATED` | 菜单、弹层、浮层 |
| `border` | `BORDER` | 主分割线 / 描边 |
| `border_subtle` | `BORDER_SUBTLE` | 弱分割线 |
| `text` | `TEXT` | 默认正文 |
| `text_muted` | `TEXT_MUTED` | 次要说明 |
| `text_disabled` | `TEXT_DISABLED` | 禁用态 |
| `accent` | `ACCENT` | 焦点环、主强调、链接感 |
| `accent_soft` | `ACCENT_SOFT` | 弱强调底 |
| `danger` | `DANGER` | 错误 / 危险操作 |
| `success` | `SUCCESS` | 成功 / 已连接等 |
| `tab_active` | `TAB_ACTIVE` | 活动 Tab 底 |
| `hover` | `HOVER` | 行/按钮悬停 |
| `selection` | `SELECTION` | 列表 / 文本选中底 |
| `icon_local` | `ICON_LOCAL` | 侧栏 Local 图标 |
| `icon_remote` | `ICON_REMOTE` | 侧栏 SSH 图标 |
| `icon_group` | `ICON_GROUP` | 侧栏 Group 图标 |

**可选第二期再加**（今日无常量硬推、不进第一版 JSON）：

| 键 | 说明 |
|----|------|
| `title_bar_bg` | CSD 标题栏底（默认同 `panel_bg`） |
| `status_bar_bg` | 状态栏底（默认同 `panel_bg`） |
| `warning` | 警告态（Info 面板等；没有则用 `accent` / 独立常量） |

### C. 终端 palette（主题包可选段）

| JSON / 字段 | 说明 |
|-------------|------|
| `terminal.background` | 终端默认背景（常跟 `bg`） |
| `terminal.foreground` | 默认前景（常跟 `text`） |
| `terminal.cursor` | 光标色（可缺省 = `accent`） |
| `terminal.ansi.black` … `white` | 标准 8 色 |
| `terminal.ansi.bright_black` … `bright_white` | 亮色 8 色 |

缺整段 `terminal`：从 UI token 推导 bg/fg，ANSI 用内置 Default 或当前 `ansi_palette` 预设。

不做（相对 Zed）：dim 档、256 色表、syntax highlight。

### D. 明确不可改（第一期 / 主题 JSON）

| 项 | 原因 |
|----|------|
| `SPACE_*` / `RADIUS_*` / `TAB_BAR_HEIGHT` / `STATUS_BAR_*` | 几何 metrics，保持编译期常量 |
| `FONT_UI*` / 终端字号 | 已有 Settings；不进主题色包 |
| Editor gutter / vim / syntax / players / icon theme | IDE 面，Loom 无 |
| 任意未列自定义键 | 忽略或校验失败提示，不进运行时 |

### E. 用户主题 JSON 示例

```json
{
  "id": "my-slate",
  "name": "My Slate",
  "appearance": "dark",
  "ui": {
    "bg": "#1a1c20ff",
    "sidebar_bg": "#14161aff",
    "panel_bg": "#1e2128ff",
    "elevated": "#262a32ff",
    "border": "#333842ff",
    "border_subtle": "#2a2e36ff",
    "text": "#e6e8ecff",
    "text_muted": "#8b909aff",
    "text_disabled": "#5c616aff",
    "accent": "#6b9bd1ff",
    "accent_soft": "#2a3a4fff",
    "danger": "#d16b6bff",
    "success": "#6bc48aff",
    "tab_active": "#2a2e36ff",
    "hover": "#2e333cff",
    "selection": "#3a4555ff",
    "icon_local": "#6bc4a0ff",
    "icon_remote": "#7a9fd4ff",
    "icon_group": "#d4a06bff"
  },
  "terminal": {
    "background": "#1a1c20ff",
    "foreground": "#e6e8ecff",
    "ansi": {
      "blue": "#74ade8ff",
      "bright_blue": "#8fbef0ff"
    }
  }
}
```

色值：`#RRGGBB` 或 `#RRGGBBAA`。未写的 `ui` / `ansi` 键回落基座（`appearance` 选 dark→`loom-dark`，light→`loom-light`）。

## 数据形状（建议）

### 运行时

```text
ThemeTokens {
  // surfaces
  bg, sidebar_bg, panel_bg, elevated,
  border, border_subtle,
  // text
  text, text_muted, text_disabled,
  // accent / state
  accent, accent_soft, danger, success,
  tab_active, hover, selection,
  icon_local, icon_remote, icon_group,
}

Theme {
  id: SharedString,          // "loom-dark"
  name: SharedString,        // 显示名
  appearance: Dark | Light,
  ui: ThemeTokens,
  terminal: Option<TerminalPalette>, // None = 从 ui + ansi_palette 推导
}
```

间距 / 半径 / 栏高：**继续**放在 `theme.rs` 常量（或 `ThemeMetrics`），不进 JSON 第一版。

### 持久化

在现有 `SettingsFile`（或等价 settings JSON）增加：

```text
theme: "loom-dark"                 // 必选，默认 loom-dark
terminal_theme: null | "…"         // null = 跟随 UI；否则独立 id / 文件名
appearance: "manual" | "system"    // 可选；system 时在 dark/light 内置包间跟随 OS
// ansi_palette: 已存在，见 TERMINAL_ANSI_PALETTE.md
```

## 实现阶段与难度

| 阶段 | 内容 | 难度 | 说明 |
|------|------|------|------|
| **0** | 自定义标题栏（CSD） | 中 | `appears_transparent: true` + 自绘 TitleBar + `WindowControlArea::{Drag,Min,Max,Close}`；与主题同色才不割裂。可先于或并行于阶段 1 |
| **1** | `Theme` / `ThemeTokens` + 访问层 | 中 | 暗色数值搬进结构体；`theme(cx).panel_bg`（或 Global）；**行为不变**。调用点迁移量大、逻辑简单 |
| **2** | 内置 light（+ 可选 HC）+ Settings 切换 | 中低 | 改 `theme_id` → 换 ActiveTheme → 观察者 `notify` |
| **3** | 终端 palette 跟随 / 可独立 | 中 | 接到 `TerminalConfig` / 已开 tab 热更新 |
| **4** | 用户 JSON 主题 | 中高 | 校验、缺省合并、错误提示、热加载 |
| **5** | 跟随系统外观 | 低～中 | 依赖阶段 0+2；听 `WindowAppearance` |
| **6** | 插件主题 | 高 | 明确排在 E1 之后 |

**总判断**：做出可切换的内置主题是 **中等工程**；难点是 **调用点迁移 + 标题栏/终端一致性**，不是调色板本身。

## 建议实施顺序

1. **CSD 标题栏**（或至少 transparent），StatusBar / TitleBar 共用 surface token。  
2. **结构体 + 访问层**：`loom-dark` = 今日常量；全库替换 `theme::COLOR` → 运行时取值（间距常量可仍 `theme::SPACE_*`）。  
3. **Settings 增加主题选择** + `loom-light`。  
4. **终端 palette 联动**（跟随 / 独立）。  
5. **JSON 用户主题** → **跟随系统** →（更后）插件。

迁移期约定：

- **禁止**新 UI 散落魔法 `Hsla { … }`，只引用 token。  
- 未完成阶段 1 前，保持现有 `pub const` 亦可，但新代码必须走同一组名字，便于一次性替换。

## GPUI / 窗口要点（阶段 0）

当前开窗（示意）：

```text
TitlebarOptions { appears_transparent: false, title: "Loom", … }
```

目标：

1. `appears_transparent: true`（macOS / Windows；Linux 用 `window_decorations: Client`）。  
2. 自绘一行 TitleBar（或与 TabBar **合并为一行**，Zed 风格，可选）。  
3. 拖拽区 / 系统按钮：`window_control_area(WindowControlArea::…)`。  
4. 背景与 `panel_bg` / `sidebar_bg` 同源，底边 `border_subtle`。

参考：gpui-component `TitleBar`、`TitlebarOptions` 文档；Windows 命中测试已支持 Drag/Min/Max/Close。

## 与现有模块的衔接

| 模块 | 改动要点 |
|------|----------|
| `shared/theme.rs` | 常量 → `ThemeTokens` 工厂（`loom_dark()` / `loom_light()`）；metrics 常量可留 |
| `model` settings / persist | 增 `theme` 字段与默认值 |
| `ui/settings.rs` | 主题下拉 / 列表 |
| 所有 `ui/*` render | 从 ActiveTheme 取色 |
| `terminal/gpui_emu` | `ColorPalette` 可替换；打开 tab / 切换主题时更新 |
| `app.rs` | CSD 开窗选项；可选注册 appearance 监听 |

## 验收标准（阶段 2 完成时）

- [ ] Settings 可在至少两套内置主题间切换，无需重启。  
- [ ] Sidebar / TabBar / StatusBar / 面板边框颜色一致跟随。  
- [ ] 原生浅色标题栏不再「浮」在深色应用上（已上 CSD，或系统栏在可接受范围内被文档标明为暂缓）。  
- [ ] 终端默认背景/前景与当前主题协调（阶段 3 可列为必须）。  
- [ ] 未知 / 损坏的 `theme` 配置回落 `loom-dark`，不崩溃。

## 明确不做的捷径

- 仅用 Windows 系统暗色标题栏「凑合」当主题系统 —— 无法对齐应用 token，Light 主题仍会裂。  
- 第一期就做完整用户 JSON + 插件 —— 契约未稳时格式会反复破。  
- 把终端 256 色动画主题与 UI 绑死 —— 保持分层，避免 UI 小改逼用户重装终端配色。
