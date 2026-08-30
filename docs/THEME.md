# 主题系统（Theme）实现方案

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[BACKLOG.md](./BACKLOG.md)（A1 / A2）。

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
| Settings 仅有 font / line numbers | 尚无 `theme` 字段 |
| 窗口 `appears_transparent: false`（原生标题栏） | 系统标题栏颜色不受应用主题控制，易与底栏/内容割裂 |

业界（VS Code / Zed / Windows Terminal）普遍做法：

- **Client-Side Decorations**：自绘标题栏，与 StatusBar 同色阶  
- **Token 契约**：surfaces / text / accent，而不是散落魔法色  
- **UI 主题与终端 palette 分两层**，可「跟随」或「独立选择」

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
- 只覆盖 **已知 token 键**；缺字段回落到所选内置基座  
- 非法 JSON / 非法色值：Settings 提示，不崩溃，保持上一有效主题

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
  ui: ThemeTokens,
  terminal: TerminalPalette, // 或 Option：None = 从 ui 推导默认 bg/fg
}
```

间距 / 半径 / 栏高：**继续**放在 `theme.rs` 常量（或 `ThemeMetrics`），不进 JSON 第一版。

### 持久化

在现有 `SettingsFile`（或等价 settings JSON）增加：

```text
theme: "loom-dark"                 // 必选，默认 loom-dark
terminal_theme: null | "…"         // null = 跟随 UI；否则独立 id / 文件名
appearance: "manual" | "system"    // 可选；system 时在 dark/light 内置包间跟随 OS
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
