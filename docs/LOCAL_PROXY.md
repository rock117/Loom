# Local shell 代理（环境变量注入）

相关文档：[ARCHITECTURE.md](./ARCHITECTURE.md)、[BACKLOG.md](./BACKLOG.md)（P3 可覆盖）。

> **状态**：**已实现**（Settings → Local Proxy；spawn 时注入）。  
> **范围**：仅 **Local** 会话；**不**覆盖 SSH 出站代理 / Jump / 端口转发。  
> **文档约定**：新增规格默认中文。

## 一句话目标

在 Settings 中配置代理模式；**新开 Local shell** 时把代理写入子进程环境变量（`HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`），使 shell 内 `git` / `npm` / `curl` 等自动走代理。

## 为何在 spawn 时注入

Local 是用户交互式 shell，无法可靠拦截用户在终端里敲的每一次 `git pull` 等命令再「失败后重试」。因此代理在 **PTY 启动时** 写入环境（Off / Manual / Auto），而不是事后包装单次外发命令。

## 模式

| Mode | 行为 |
|------|------|
| **Off** | 不注入任何代理相关 env（默认） |
| **Auto** | `detect_proxy()`：先查进程环境变量，再读 OS 系统代理；有则注入 |
| **Manual** | 使用用户填写的 URL；按 scheme 决定注入哪些变量 |

可选：**No proxy** 文本 → 始终可设 `NO_PROXY`（三种模式都可带；Off 时若只填 NO_PROXY 也可注入该键，或 Off 时忽略——实现取：仅 Auto/Manual 时写入 NO_PROXY，避免 Off 仍改环境）。

## 探测顺序（Auto）

1. 环境变量：`HTTPS_PROXY` → `HTTP_PROXY` → `ALL_PROXY`（大小写均查）  
2. OS 系统代理：  
   - **Windows**：`HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`（`ProxyEnable` + `ProxyServer`）  
   - **macOS**：`networksetup`（HTTPS → SOCKS → HTTP）  
   - **Linux**：GNOME `gsettings`（SOCKS → HTTP）

## 注入规则

| URL 类型 | 注入 |
|----------|------|
| `http://` / `https://` / 无 scheme（补成 `http://`） | `HTTP_PROXY` + `HTTPS_PROXY`（及小写副本，兼容老工具） |
| `socks5://` / `socks://` | `ALL_PROXY` + `all_proxy` |
| No proxy 非空 | `NO_PROXY` + `no_proxy` |

**仅 Local**：SSH / SFTP 连接路径不读此设置。

## Settings UI

- 分区：**LOCAL PROXY**（或放在 Terminal 下）  
- Mode chips：`off` / `auto` / `manual`  
- Manual：URL 输入框  
- No proxy：可选输入（`localhost,127.0.0.1,.corp`）  
- 脚注：仅对新开的 Local tab 生效；已开会话需重开或 Reconnect。

## 持久化（`settings.json`）

```json
{
  "local_proxy_mode": "off",
  "local_proxy_url": null,
  "local_proxy_no_proxy": null
}
```

`local_proxy_mode`: `"off" | "auto" | "manual"`。缺省字段按 Off。

## 非目标

- SSH 经 HTTP/SOCKS 出站、Jump host（另案）  
- 已打开 Local tab 热更新 env  
- 包装/拦截 shell 内命令失败后再注入重试  
- WinHTTP「自动配置脚本」PAC 完整解析（MVP 只读静态 ProxyServer）

## 代码路径（实现）

| 模块 | 职责 |
|------|------|
| `src/session/local_proxy.rs` | 探测 + 解析为 env 列表 |
| `src/model/workspace.rs` | `SettingsFile` 字段 |
| `src/session/local.rs` | spawn 时 `cmd.env(...)` |
| `src/ui/settings.rs` | UI |
| `src/ui/tab_manager.rs` | spawn_local 传入当前 settings |

## 验收

- [ ] Off：Local 内 `echo $env:HTTP_PROXY`（pwsh）为空（或未由 Loom 设置）。  
- [ ] Manual + `http://127.0.0.1:7890`：新 Local 可见对应 env。  
- [ ] Auto：系统代理开启时能侦测并注入；关闭则不注入。  
- [ ] SSH tab 不受影响。  
- [ ] 改设置后须新开 Local（或 Reconnect）才生效。
