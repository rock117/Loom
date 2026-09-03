# Zed 主题系统研究（对照 Loom）

相关文档：[THEME.md](./THEME.md)、[ARCHITECTURE.md](./ARCHITECTURE.md)、[DECISIONS.md](./DECISIONS.md)、[TERMINAL_ANSI_PALETTE.md](./TERMINAL_ANSI_PALETTE.md)。

> **性质**：本地 Zed 源码（`C:\rock\coding\code\opensource\rust\zed`）的**架构/行为研究**，给 Loom 主题实现当对照。  
> **许可**：Zed `crates/theme`、`theme_settings`、`theme_selector`、`theme_extension` 等为 **GPL**。按 [DECISIONS.md](./DECISIONS.md)：**只学结构，不粘贴 Zed 源码进 Loom**。  
> **状态**：研究完成；Loom 可切换主题仍见 [THEME.md](./THEME.md)（尚未实现）。

## 一句话

Zed 的主题不是「换一套 `const` 颜色」，而是：

**JSON 家族文件 → 注册表 → 当前 `GlobalTheme` → UI 一律 `cx.theme().colors()` 取色；设置变更时替换 Global 并 `refresh_windows()`。**

缺字段用 **Light/Dark 基座色** 填满；设置里还可以再叠一层 `theme_overrides`。

---

## 模块怎么拆（Zed crate）

| Crate | 职责 | Loom 对照 |
|-------|------|-----------|
| `theme` | 运行时 `Theme` / `ThemeColors`、注册表、`GlobalTheme`、`ActiveTheme`、编译期 fallback、图标主题 | 将来 `shared/theme.rs` + 小注册表；**不要**搬 Zed 那张超大 token 表 |
| `theme_settings` | 读 `settings.json`、加载内置/用户 JSON、观察 SettingsStore、热切换 | Loom `SettingsFile` + Settings UI |
| `theme_selector` | 命令面板式列表；**上下键实时换主题**，Enter 写入设置 | Settings 下拉即可；预览可选 |
| `theme_importer` | VS Code 主题 → Zed JSON | 非目标 |
| `theme_extension` | 扩展市场装主题包 | 排在插件 E1 之后 |
| `assets/themes/*.json` | 内置包（One / Ayu / Gruvbox…） | 第一期 `loom-dark` / `loom-light` 结构体即可 |

启动顺序（概念）：

1. `theme::init`：装 `ThemeRegistry`、`SystemAppearance`、`FontFamilyCache`；先塞 **编译期 fallback 暗色**，保证测试/首帧永远有主题。  
2. `theme_settings::init`：从资源列出 `themes/**/*.json` 填注册表；按设置算出当前主题写入 `GlobalTheme`。  
3. `observe_global::<SettingsStore>`：主题名或 overrides 变了 → `reload_theme` → `cx.refresh_windows()`。

---

## 运行时对象

```text
ThemeFamily          一组变体（如 "One" 含 One Dark / One Light）
  └── Theme          id, name, appearance(Light|Dark), styles
        └── ThemeStyles
              colors     UI + editor + terminal ANSI（一张大表）
              status     error / warning / success…
              syntax     高亮 capture → 颜色/斜体/字重
              accents    循环强调色（协作光标、彩虹括号）
              player     多光标/协作者
              system     系统色桥接
              window_background_appearance
```

访问路径（UI 渲染）：

```text
cx.theme()                 // ActiveTheme → Arc<Theme>
  .colors().panel_background
  .status().error
  .syntax() …
```

`Theme` 存在 **App Global**（`GlobalTheme`），不是每个 View 自己缓存一份色表。换主题 = 换 Arc + 刷新窗口。

另有：

- **`ThemeRegistry`**：`name → Arc<Theme>`（及 icon theme）。内置 JSON、用户目录、扩展都往这里插。  
- **`SystemAppearance`**：从 GPUI `WindowAppearance` 映射 Light/Dark（含 Vibrant）。  
- **Icon theme**：独立第二套（文件树图标），与配色主题分开选。Loom 第一期可忽略。

---

## JSON 形状（用户/内置文件）

内置示例：`assets/themes/one/one.json`。

```text
{
  "$schema": "…/themes/v0.2.0.json",
  "name": "One",          // family
  "author": "…",
  "themes": [
    {
      "name": "One Dark",
      "appearance": "dark",
      "style": {
        "background": "#3b414dff",
        "text": "#dce0e5ff",
        "panel.background": "#2f343eff",
        "tab.active_background": "…",
        "editor.background": "…",
        "terminal.background": "…",
        "terminal.ansi.blue": "…",
        "syntax": { "comment": { "color": "…", "font_style": "italic" } }
      }
    },
    { "name": "One Light", "appearance": "light", "style": { … } }
  ]
}
```

要点：

- **Family 一个文件、多 appearance**，而不是 dark/light 两个无关文件。  
- 键名用 **点号**（`panel.background`），Rust 字段是 snake_case，serde 做映射。  
- 值是 `#RRGGBBAA` 字符串，解析成 `Hsla`。  
- `style` 里大量字段是 **可选的**。

### Refine（缺省合并）——对 Loom 最有用的一点

加载时大致是：

1. 按 `appearance` 取 **完整基座**：`ThemeColors::dark()` 或 `::light()`。  
2. JSON 只填的键变成 `ThemeColorsRefinement { text: Some(…), ..Default }`。  
3. `refine(&overrides)` 覆盖基座。  
4. 再跑少量派生默认（例如没写 selection 背景就从 player 色半透明推）。

因此 **用户主题不必列出全部 token**；非法/缺键不崩，静默回落。Loom [THEME.md](./THEME.md) 的「缺字段回落内置基座」与此同构，实现时用手写 `Option` merge 即可，不必引入 Zed 的 `refineable` crate。

---

## 设置如何选主题

用户设置（摘自 Zed 文档行为）：

```json
{
  "theme": {
    "mode": "system",
    "light": "One Light",
    "dark": "One Dark"
  }
}
```

也允许写成单个字符串 `"theme": "One Dark"`（静态）。

解析：

- `mode`: `system` | `light` | `dark`  
- `system` 时用 `SystemAppearance` 在 light/dark **两个主题名**之间选  
- 找不到名字：log，回落到默认暗色（扩展未加载完时不吵）

**`theme_overrides`**：按主题名再盖一层（改某一个 token、改 comment 斜体）。这是「用户补丁」，不是新主题文件。Loom 第一期可不做；JSON 用户主题已覆盖大部分需求。

字体、`ui_density` 也挂在 theme settings 上，但它们**不是配色 JSON 的一部分**。Loom 间距/字号继续当 metrics 常量即可。

切换后：`GlobalTheme::update_theme` + **`cx.refresh_windows()`**（整窗重绘，不重启进程）。

---

## 选择器 UX

`theme_selector`：

- 列出注册表里所有主题（可按 Dark/Light 过滤）。  
- **导航即预览**：改当前 Global，不立刻写盘。  
- **Enter 才写入** `settings.json`。  
- Dismiss 恢复进入前的主题。

Loom Settings 用下拉切换也能满足；若以后要「逛主题」，可复用「预览 / 确认 / 取消」这套状态机，不必做成 Zed 命令面板。

---

## 终端配色在 Zed 里怎么挂

Zed **没有**单独的「终端主题 ID」。ANSI 16 色 + dim/bright + `terminal.background/foreground` 都在 **同一份** `style` 里。

换 UI 主题 = 换编辑器 + 换终端 palette。用户若只要改蓝字，用 `theme_overrides` 或另存一个 family 变体。

Loom 现状相反：

| | Zed | Loom 现状 |
|--|-----|-----------|
| UI 色 | 运行时 `ThemeColors` | 编译期 `theme.rs` 常量 |
| 终端 ANSI | 主题 JSON 的 `terminal.ansi.*` | Settings **独立** `AnsiPalette` 预设（[TERMINAL_ANSI_PALETTE.md](./TERMINAL_ANSI_PALETTE.md)） |

[THEME.md](./THEME.md) 已规划「默认可跟随 UI，允许独立」。对标 Zed 时建议：

- **跟随**：从 ActiveTheme 填 `ColorPalette`（已有 `set_ansi_palette` 热更新路径）。  
- **独立**：保留现有 ANSI 预设，覆盖主题里的终端段。  
- **不要**第一期就做 dim/bright 各 8 色 + syntax map；Loom 不是编辑器。

---

## 标题栏 / CSD

Zed token 里有 `title_bar.background` / `title_bar.inactive_background`，配合自绘标题栏（CSD）。这与 [THEME.md](./THEME.md) 阶段 0 一致：不换 CSD，Light 主题会和系统浅色标题栏打架。

`ClientDecorationsExt`（圆角/阴影）是窗口几何，不是配色；Loom 可后做。

---

## 数据从哪来

```text
编译期 fallback Theme          测试 / 资源失败时的底
     +
内置 assets/themes/*.json      随二进制
     +
~/.config/zed/themes/*.json    本地用户包（下次启动可见）
     +
扩展商店主题                   动态 insert / remove
```

注册表 `get(name)` 失败不 panic。Loom 第一期：结构体工厂 + Settings `theme_id` 足够；用户 JSON 放到阶段 4。

---

## 和 Loom 现有规格怎么对齐

[THEME.md](./THEME.md) 里的产品分层（UI tokens / 终端 palette / chrome 几何）**仍然正确**；**可改设置与 token 全表**见该文「Loom 可修改的主题设置」。Zed 研究补充的是 **机制**，不是要复制 token 数量。

建议 Loom **采纳**的机制：

1. **单一 ActiveTheme（Global 或 Workspace Entity）**；render 只读 token，禁止新代码写死 `Hsla { … }`。  
2. **Light/Dark 两套完整基座**；JSON/用户覆盖用 `Option` merge。  
3. **换主题 = 换 Arc + notify / refresh**；已开终端走现有 `TabManager::set_ansi_palette`。  
4. **设置**用 `theme_id`，可选 `appearance: manual | system`（系统外观映射 Dark/Light 内置包）。  
5. **Family 可选**：`loom-dark` / `loom-light` 两个 id 即可，不必先做 family 文件。

建议 **明确不抄** 的：

- 上百个 `ThemeColors` 字段（editor gutter、vim mode、minimap…）  
- Syntax highlight / player / icon theme  
- `refineable` 过程宏、GPL crate 布局  
- VS Code importer、扩展市场  
- 选择器实时预览（可后做）

Zed 字段 → Loom 现有常量的**概念映射**（实现时用 Loom 自己的值）：

| Zed（概念） | Loom 今天 |
|-------------|-----------|
| `background` | `BG` |
| `panel.background` / `surface.background` | `PANEL_BG` / `SIDEBAR_BG` |
| `elevated_surface.background` | `ELEVATED` |
| `border` / `border.variant` | `BORDER` / `BORDER_SUBTLE` |
| `text` / `text.muted` / `text.disabled` | `TEXT` / `TEXT_MUTED` / `TEXT_DISABLED` |
| `element.hover` / `tab.active_background` | `HOVER` / `TAB_ACTIVE` |
| `text.accent` | `ACCENT` |
| `terminal.*` | `ColorPalette` / `AnsiPalette` |

---

## 推荐实现顺序（相对 THEME.md 微调）

Zed 能热切换，是因为 **调用点从第一天就走 `cx.theme()`**。Loom 最大成本仍是把 `theme::PANEL_BG` 换成运行时。

1. **结构体 + 访问层**（THEME 阶段 1）：`loom_dark()` 数值与今天常量一致；可先留 `pub const` 别名指向 ActiveTheme，逐步删。  
2. **Settings 切 Dark/Light** + notify。  
3. **终端跟随**（独立 ANSI 预设作为覆盖）。  
4. CSD 标题栏（Light 主题才刚需）。  
5. 用户 JSON（同一套 token 键、缺省 merge）。  
6. `appearance: system`。

不要为对齐 Zed 先做 JSON schema / 主题选择器 / 插件。

---

## 本地源码索引（只读，勿拷进仓库）

| 内容 | 路径 |
|------|------|
| `Theme` / `GlobalTheme` / `init` | `crates/theme/src/theme.rs` |
| 注册表 | `crates/theme/src/registry.rs` |
| UI+终端色表 | `crates/theme/src/styles/colors.rs` |
| 编译期 fallback | `crates/theme/src/fallback_themes.rs` |
| 设置观察与 JSON 加载 | `crates/theme_settings/src/theme_settings.rs` |
| `theme` / `mode` / `light` / `dark` | `crates/theme_settings/src/settings.rs` |
| 内置 JSON | `assets/themes/one/one.json` |
| 用户文档 | `docs/src/themes.md` |
| 选择器 | `crates/theme_selector/src/theme_selector.rs` |

---

## 验收对照（研究用，非实现清单）

读懂 Zed 主题后，Loom 实现应能回答：

- [ ] 换主题要不要重启？（Zed：不要）  
- [ ] JSON 少写 90% 的键会怎样？（Zed：基座补齐）  
- [ ] 终端和 UI 是否同一份主题？（Zed：是；Loom：跟随 + 可选独立）  
- [ ] 未知主题名会不会崩？（Zed：回落默认暗色）
