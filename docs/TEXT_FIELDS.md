# 文本输入框标准（选区 / 剪贴板 / IME）

Loom 自定义输入（非系统原生 `<input>`）的统一约定。  
相关硬坑：[TERMINAL_IME.md](./TERMINAL_IME.md)、[HARD_PROBLEMS.md](./HARD_PROBLEMS.md)（UI 卡死）。

---

## 产品要求（凡输入框）

每个可编辑文本表面至少支持：

| 能力 | 最低行为 |
|------|----------|
| **选区** | 鼠标拖选；Shift+方向键 / Home / End；Ctrl+A 全选 |
| **复制 / 剪切** | Ctrl+C / Ctrl+X（有选区拷选区；无选区可拷全文，与现有表单一致即可） |
| **粘贴** | Ctrl+V、Shift+Insert；粘贴替换当前选区 |
| **编辑** | 插入、Backspace、Delete 尊重选区 |
| **中文 IME** | 组字上屏进该字段（见 IME 文档；焦点与 `handle_input` 必须一致） |
| **可见反馈** | 选区高亮 + 闪烁光标（用 `RenameEdit::into_element*`） |

**禁止：** 只用 `String` + `push`/`pop` + 画纯文本——这是「搜不了选区 / 拷不了 / 中文丢」的温床（Find 条、早期 Settings 自定义行都犯过）。

---

## 标准实现

1. **状态：** `crate::ui::rename_edit::RenameEdit`（`cursor` / `anchor` / `insert` / `selected_text` / …）。  
2. **渲染：** `into_element()` 或 `into_element_bare()`（外框自管时）；密码用 `into_element_bare_masked()`。  
3. **键盘：** 参考 `ssh_form` / `context_panel` 路径——Ctrl+A/C/V/X、方向键+Shift、`typed_text_from_keystroke`。  
4. **鼠标：** 命中测字索引（`char_index_at_x` + 字段 `bounds` canvas）、按下设 caret、拖动扩展选区。  
5. **新建字段 checklist：**

```text
[ ] RenameEdit，不是裸 String
[ ] 选区可见（into_element*）
[ ] Ctrl+A / C / X / V + Shift+Insert
[ ] 鼠标拖选
[ ] 若另有 FocusHandle：handle_input 绑同一 handle（CJK）
[ ] 未在 UI 线程做阻塞 I/O（HARD_PROBLEMS）
```

---

## 合规与债务（审计）

| 表面 | 状态 |
|------|------|
| SSH 表单字段 / 转发编辑 | ✅ `RenameEdit` |
| Context Files 路径栏、过滤器、提示框、Temporary 转发 | ✅ |
| Sidebar / Tab 重命名 | ✅ |
| Transfer settings 表单 | ✅（经 Context prompt） |
| 终端 Ctrl+F 查询 | ✅ `RenameEdit` + Find 打开时 IME 绑 find 焦点 |
| 密码弹窗 `PasswordPrompt` | ⚠️ 有粘贴/整框拷贝，**无字符选区** → 应迁 `RenameEdit` |
| Settings 内联 Shell / Font / Proxy 行 | ⚠️ 裸 `String` push/pop，**无选区** → 应迁 `RenameEdit` |

新功能不得再增加「裸 String 输入」行；改旧债时优先迁 `RenameEdit`。

---

## 与 IME / 卡死的交界

- 输入框若 **抢走** 终端焦点：必须把 `window.handle_input` 指到该框的 `FocusHandle`，提交写入该 `RenameEdit`，否则中文静默丢失（Find 事故）。  
- 输入逻辑不得在 `render`/paint 里锁 `term` 或做网络/磁盘；卡死清单见 HARD_PROBLEMS。

---

## 相关代码

| 路径 | 职责 |
|------|------|
| `src/ui/rename_edit.rs` | 选区模型 + 渲染 |
| `src/ui/ssh_form.rs` | 完整表单参考实现 |
| `src/terminal/gpui_emu/view/find.rs` | Find 查询 + 选区 |
| `src/terminal/gpui_emu/view/mod.rs` | Find 打开时 IME 分流 |
