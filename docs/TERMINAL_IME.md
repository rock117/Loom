# 终端 IME（中文输入）与回车

知识笔记：记录 Loom 终端无法输入中文、以及接入 IME 后回车失效的排查过程与最终模型。  
**不属于架构规格**；平台坑索引见 [HARD_PROBLEMS.md](./HARD_PROBLEMS.md)。

---

## 现象

1. **只能打英文**：切换到中文输入法后，拼音/候选确认后的汉字进不了 shell。  
2. **接入 IME 后回车没反应**：中英文输入后按 Enter，命令不提交。

---

## 根因（两层）

### 1. 缺 IME 接入

GPUI 在 Windows 上通过 `WM_IME_COMPOSITION` / `WM_CHAR` 把组字结果交给当前窗口的 **`InputHandler`**（`Window::handle_input`）。

Loom 终端原先只监听 **`KeyDown`**，用 `keystroke_to_bytes` 写 PTY：

- 英文：`key_char` 有值 → 能进 PTY  
- 中文：组字结果走 IME 消息 → **没有 InputHandler 就丢弃**

### 2. 回车与「可打印字符」路径搅在一起

Windows 侧要点：

| 消息 | 行为 |
|------|------|
| `WM_KEYDOWN` 且应用 **已处理**（`stop_propagation`） | **不** `TranslateMessage`，无后续 `WM_CHAR` |
| `WM_KEYDOWN` 未处理 | `TranslateMessage` → 可能产生 `WM_CHAR` / 交给 IME |
| 正在 composition（`marked_text_range` 为 `Some`） | KeyDown **不派发到应用**，只 `TranslateMessage` |
| `WM_CHAR` 解析 | **过滤控制字符**（`!c.is_control()`）→ `\r` / `\t` **不会**进 InputHandler |

因此：

- **Enter 必须走 KeyDown → 直接写 `\r`**，不能指望 `WM_CHAR`。  
- **普通字母/汉字**应走 InputHandler，KeyDown 不要再写一遍（否则英文双字符，或抢掉 IME）。

中间失败做法：凡是 `key_char.is_some()` 都 defer 给 IME。Enter 在部分路径上带有 `key_char="\r"`，被 defer 后 `WM_CHAR` 又丢掉 `\r` → **回车静默失败**。

---

## 解决思路（双管道）

终端输入拆成两条互不抢夺的管道：

```
KeyDown
  └─ try_keystroke / to_esc_str
       ├─ 有映射（enter、tab、方向键、Ctrl+C…）→ 写 PTY，stop_propagation
       └─ None（普通可打印）→ 不处理、不 stop
            └─ Windows TranslateMessage
                 ├─ WM_CHAR / IME commit → InputHandler.replace_text_in_range → 写 PTY
                 └─ IME preedit → replace_and_mark_text_in_range（只记 marked，不写 PTY）
```

### Loom 对应改动

| 组件 | 做法 |
|------|------|
| `TerminalView` | 实现 `EntityInputHandler`；canvas paint 时 `window.handle_input(..., ElementInputHandler::new(...))` |
| `keystroke_to_bytes` | **只**返回 Escape/控制序列；**去掉** `key_char` / 单字符 fallback（空格明文也改走 IME） |
| `on_key_down` | 有 bytes → 写 PTY + `stop_propagation`；否则交给 IME；Enter/Esc 时清掉卡住的 `ime_marked` |
| `replace_text_in_range` | 提交文本写 PTY（UTF-8，非 bracketed paste） |
| `replace_and_mark_text_in_range` | 只保存 preedit；**不**写 PTY |
| `marked_text_range` | 有非空 preedit 才 `Some`（避免空 marked 让 Windows 一直以为在组字，从而吞掉 Enter 的 KeyDown） |

### 使用注意（产品层）

中文输入法下，**第一次 Enter 常用于上屏候选**，由 IME 消费，不一定会再给终端一条 `\r`。上屏后若要执行命令，可能需要 **再按一次 Enter**（常见终端行为）。

---

## 失败尝试（勿重复）

| 尝试 | 结果 |
|------|------|
| 只靠 KeyDown + `key_char` | 中文永远进不来 |
| 凡 `key_char.is_some()` 都 defer 给 IME | Enter 的 `\r` 被 defer 后又被 `WM_CHAR` 过滤 → 回车失效 |
| KeyDown 仍写可打印 + 同时接 InputHandler | 英文双击字符 / 与 IME 抢键 |
| 依赖 `WM_CHAR` 传递 Enter | 控制字符被 GPUI Windows 后端滤掉 |

---

## 相关代码

| 路径 | 职责 |
|------|------|
| `src/terminal/gpui_emu/view/mod.rs` | `EntityInputHandler`、`on_key_down`、`handle_input`、IME 状态 |
| `src/terminal/gpui_emu/input.rs` | `keystroke_to_bytes`（仅 Escape/控制） |
| GPUI `platform/windows/events.rs` | `WM_IME_*`、`WM_CHAR`、`marked_text_range` 与 TranslateMessage |

---

## 规则（以后改输入时先看）

1. **可打印 / CJK → InputHandler**；**Enter / Tab / 方向键 / Ctrl 序列 → KeyDown**。  
2. KeyDown 一旦写入 PTY，必须 **`stop_propagation`**，避免再走 `WM_CHAR` 重复。  
3. **不要用 `key_char.is_some()` 判断「交给 IME」**——Enter 也可能带 `key_char`。  
4. `marked_text_range` 只在真正有 preedit 时返回 `Some`，否则 Windows 会吞掉后续 KeyDown（含 Enter）。
