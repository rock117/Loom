//! Detect and resolve proxy env vars for **Local** shell spawn.
//!
//! Auto probe order: process env → OS system proxy.
//! Inject at PTY spawn time (interactive shells cannot reliably retry per-command).

use serde::{Deserialize, Serialize};

/// User choice in Settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalProxyMode {
    #[default]
    Off,
    /// Env vars, then OS system proxy.
    Auto,
    /// Use [`crate::model::SettingsFile::local_proxy_url`].
    Manual,
}

impl LocalProxyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyUrl {
    pub url: String,
    pub is_socks: bool,
}

impl ProxyUrl {
    #[cfg(not(windows))]
    fn http(url: String) -> Self {
        Self {
            url,
            is_socks: false,
        }
    }

    #[cfg(not(windows))]
    fn socks(url: String) -> Self {
        Self {
            url,
            is_socks: true,
        }
    }

    pub fn from_url(url: String) -> Self {
        let is_socks = url.to_ascii_lowercase().starts_with("socks");
        Self { url, is_socks }
    }
}

/// Resolve `(key, value)` pairs to inject into a Local shell, or empty if Off / nothing found.
pub fn proxy_env_vars(
    mode: LocalProxyMode,
    manual_url: Option<&str>,
    no_proxy: Option<&str>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if matches!(mode, LocalProxyMode::Off) {
        return out;
    }

    let proxy = match mode {
        LocalProxyMode::Off => None,
        LocalProxyMode::Manual => {
            let Some(url) = manual_url.map(str::trim).filter(|s| !s.is_empty()) else {
                return out;
            };
            Some(normalize_manual(url))
        }
        LocalProxyMode::Auto => detect_proxy(),
    };

    if let Some(p) = proxy {
        if p.is_socks {
            out.push(("ALL_PROXY".into(), p.url.clone()));
            out.push(("all_proxy".into(), p.url));
        } else {
            out.push(("HTTP_PROXY".into(), p.url.clone()));
            out.push(("http_proxy".into(), p.url.clone()));
            out.push(("HTTPS_PROXY".into(), p.url.clone()));
            out.push(("https_proxy".into(), p.url));
        }
    }

    if let Some(np) = no_proxy.map(str::trim).filter(|s| !s.is_empty()) {
        out.push(("NO_PROXY".into(), np.to_string()));
        out.push(("no_proxy".into(), np.to_string()));
    }

    out
}

fn normalize_manual(url: &str) -> ProxyUrl {
    let url = if url.contains("://") {
        url.to_string()
    } else {
        format!("http://{url}")
    };
    ProxyUrl::from_url(url)
}

/// Probe: env → OS system proxy.
pub fn detect_proxy() -> Option<ProxyUrl> {
    let env_names = [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ];
    for name in &env_names {
        if let Ok(val) = std::env::var(name) {
            let val = val.trim().to_string();
            if !val.is_empty() {
                return Some(ProxyUrl::from_url(val));
            }
        }
    }
    detect_system_proxy()
}

#[cfg(windows)]
fn detect_system_proxy() -> Option<ProxyUrl> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            KEY_READ,
        )
        .ok()?;

    let enabled: u32 = key.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }

    let proxy_server: String = key.get_value("ProxyServer").ok()?;
    let proxy_server = proxy_server.trim().to_string();
    if proxy_server.is_empty() {
        return None;
    }

    let raw = extract_proxy_for_protocol(&proxy_server, "https")
        .or_else(|| extract_proxy_for_protocol(&proxy_server, "http"))
        .unwrap_or_else(|| proxy_server.clone());

    let url = if raw.contains("://") {
        raw
    } else {
        format!("http://{raw}")
    };

    Some(ProxyUrl::from_url(url))
}

#[cfg(windows)]
fn extract_proxy_for_protocol(proxy_server: &str, protocol: &str) -> Option<String> {
    for part in proxy_server.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(&format!("{protocol}=")) {
            let rest = rest.trim().to_string();
            if !rest.is_empty() {
                return Some(if rest.contains("://") {
                    rest
                } else {
                    format!("http://{rest}")
                });
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn detect_system_proxy() -> Option<ProxyUrl> {
    detect_macos_proxy().or_else(detect_linux_proxy)
}

#[cfg(not(windows))]
fn query_networksetup(iface: &str, flag: &str, scheme: &str) -> Option<String> {
    let out = std::process::Command::new("networksetup")
        .args([flag, iface])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut enabled = false;
    let mut server = String::new();
    let mut port = String::new();
    for line in text.lines() {
        if line.starts_with("Enabled:") {
            enabled = line.contains("Yes");
        } else if line.starts_with("Server:") {
            server = line["Server:".len()..].trim().to_string();
        } else if line.starts_with("Port:") {
            port = line["Port:".len()..].trim().to_string();
        }
    }
    if enabled && !server.is_empty() && port != "0" {
        Some(format!("{scheme}://{server}:{port}"))
    } else {
        None
    }
}

#[cfg(not(windows))]
fn detect_macos_proxy() -> Option<ProxyUrl> {
    let interfaces = ["Wi-Fi", "Ethernet", "en0", "en1"];
    let proxy_types: &[(&str, &str)] = &[
        ("-getsecurewebproxy", "https"),
        ("-getsocksfirewallproxy", "socks5"),
        ("-getwebproxy", "http"),
    ];
    for iface in &interfaces {
        for (flag, scheme) in proxy_types {
            if let Some(url) = query_networksetup(iface, flag, scheme) {
                return Some(if *scheme == "socks5" {
                    ProxyUrl::socks(url)
                } else {
                    ProxyUrl::http(url)
                });
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn detect_linux_proxy() -> Option<ProxyUrl> {
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy", "mode"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mode = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if mode != "'manual'" {
        return None;
    }
    if let Some(proxy) = detect_linux_socks_proxy() {
        return Some(proxy);
    }
    detect_linux_http_proxy()
}

#[cfg(not(windows))]
fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let val = String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_matches('\'')
        .to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

#[cfg(not(windows))]
fn detect_linux_socks_proxy() -> Option<ProxyUrl> {
    let host = gsettings_get("org.gnome.system.proxy.socks", "host")?;
    let port = gsettings_get("org.gnome.system.proxy.socks", "port")?;
    if port == "0" {
        return None;
    }
    Some(ProxyUrl::socks(format!("socks5://{host}:{port}")))
}

#[cfg(not(windows))]
fn detect_linux_http_proxy() -> Option<ProxyUrl> {
    let host = gsettings_get("org.gnome.system.proxy.http", "host")?;
    let port = gsettings_get("org.gnome.system.proxy.http", "port")?;
    if port == "0" {
        return None;
    }
    Some(ProxyUrl::http(format!("http://{host}:{port}")))
}
