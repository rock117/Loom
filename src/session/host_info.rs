//! Host snapshot for the Context panel Info tab (Local via sysinfo, SSH via remote probe).

use anyhow::{Context, Result, bail};

/// One-shot host metrics for display (manual refresh only).
#[derive(Debug, Clone, Default)]
pub struct HostSnapshot {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub cpu_model: String,
    pub cpu_cores: u32,
    /// 0..=100 when available (Local after a short sample).
    pub cpu_usage_pct: Option<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub disk_mount: String,
    pub load: Option<String>,
    pub uptime_secs: u64,
}

impl HostSnapshot {
    pub fn mem_ratio(&self) -> f32 {
        if self.mem_total == 0 {
            0.0
        } else {
            (self.mem_used as f64 / self.mem_total as f64) as f32
        }
    }

    pub fn disk_ratio(&self) -> f32 {
        if self.disk_total == 0 {
            0.0
        } else {
            (self.disk_used as f64 / self.disk_total as f64) as f32
        }
    }
}

/// Collect Local host info on a background thread (may sleep briefly for CPU %).
pub fn collect_local() -> Result<HostSnapshot> {
    use sysinfo::{Disks, System};

    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();
    // Second sample so usage % is meaningful.
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_all();

    let cpu_usage = if sys.cpus().is_empty() {
        None
    } else {
        let sum: f32 = sys.cpus().iter().map(|c| c.cpu_usage()).sum();
        Some(sum / sys.cpus().len() as f32)
    };
    let cpu_model = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into());
    let cpu_cores = sys.cpus().len() as u32;

    let disks = Disks::new_with_refreshed_list();
    let (disk_total, disk_used, disk_mount) = pick_primary_disk(&disks);

    let load = {
        let la = System::load_average();
        if la.one == 0.0 && la.five == 0.0 && la.fifteen == 0.0 {
            None
        } else {
            Some(format!("{:.2} {:.2} {:.2}", la.one, la.five, la.fifteen))
        }
    };

    let os = System::long_os_version()
        .or_else(|| System::name())
        .unwrap_or_else(|| "Unknown OS".into());
    let kernel = System::kernel_version().unwrap_or_default();
    let hostname = System::host_name().unwrap_or_else(|| "—".into());

    Ok(HostSnapshot {
        hostname,
        os,
        kernel,
        cpu_model,
        cpu_cores,
        cpu_usage_pct: cpu_usage,
        mem_used: sys.used_memory(),
        mem_total: sys.total_memory(),
        disk_used,
        disk_total,
        disk_mount,
        load,
        uptime_secs: System::uptime(),
    })
}

fn pick_primary_disk(disks: &sysinfo::Disks) -> (u64, u64, String) {
    let mut best: Option<(u64, u64, String)> = None;
    for disk in disks.list() {
        let total = disk.total_space();
        if total == 0 {
            continue;
        }
        let avail = disk.available_space();
        let used = total.saturating_sub(avail);
        let mount = disk.mount_point().to_string_lossy().into_owned();
        // Prefer system root: `/` or `C:\`.
        let is_root = mount == "/"
            || mount.eq_ignore_ascii_case("C:\\")
            || mount.eq_ignore_ascii_case("C:/");
        if is_root {
            return (total, used, mount);
        }
        let replace = match &best {
            None => true,
            Some((t, _, _)) => total > *t,
        };
        if replace {
            best = Some((total, used, mount));
        }
    }
    best.unwrap_or((0, 0, "—".into()))
}

/// Shell script run on remote SSH hosts (stdout KEY=value lines).
pub fn remote_probe_script() -> &'static str {
    r#"printf 'OS=%s\n' "$(uname -s 2>/dev/null || echo unknown)"
printf 'KERNEL=%s\n' "$(uname -r 2>/dev/null || echo)"
printf 'HOSTNAME=%s\n' "$(hostname 2>/dev/null || uname -n 2>/dev/null || echo)"
printf 'CPU_CORES=%s\n' "$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || echo 0)"
printf 'CPU_MODEL=%s\n' "$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | sed 's/^ *//' || sysctl -n machdep.cpu.brand_string 2>/dev/null || echo)"
if command -v free >/dev/null 2>&1; then
  free -b 2>/dev/null | awk '/^Mem:/{printf "MEM_TOTAL=%s\nMEM_USED=%s\n",$2,$3}'
elif [ -r /proc/meminfo ]; then
  awk '/MemTotal:/{t=$2*1024}/MemAvailable:/{a=$2*1024}END{printf "MEM_TOTAL=%s\nMEM_USED=%s\n",t,t-a}' /proc/meminfo
fi
df -B1 -P / 2>/dev/null | awk 'NR==2{printf "DISK_TOTAL=%s\nDISK_USED=%s\nDISK_MOUNT=%s\n",$2,$3,$6}'
if [ -r /proc/loadavg ]; then
  awk '{printf "LOAD=%s %s %s\n",$1,$2,$3}' /proc/loadavg
fi
if [ -r /proc/uptime ]; then
  awk '{printf "UPTIME=%d\n",int($1)}' /proc/uptime
elif command -v sysctl >/dev/null 2>&1; then
  # Best-effort on BSD/macOS
  boot=$(sysctl -n kern.boottime 2>/dev/null | awk -F'[=,]' '{print $2}' | tr -d ' ')
  now=$(date +%s)
  if [ -n "$boot" ] && [ -n "$now" ]; then printf 'UPTIME=%s\n' "$((now-boot))"; fi
fi
"#
}

pub fn parse_remote_probe(stdout: &str) -> Result<HostSnapshot> {
    let mut snap = HostSnapshot::default();
    let mut saw = false;
    for line in stdout.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        saw = true;
        let v = v.trim();
        match k.trim() {
            "OS" => snap.os = v.to_string(),
            "KERNEL" => snap.kernel = v.to_string(),
            "HOSTNAME" => snap.hostname = v.to_string(),
            "CPU_CORES" => snap.cpu_cores = v.parse().unwrap_or(0),
            "CPU_MODEL" => snap.cpu_model = v.to_string(),
            "MEM_TOTAL" => snap.mem_total = v.parse().unwrap_or(0),
            "MEM_USED" => snap.mem_used = v.parse().unwrap_or(0),
            "DISK_TOTAL" => snap.disk_total = v.parse().unwrap_or(0),
            "DISK_USED" => snap.disk_used = v.parse().unwrap_or(0),
            "DISK_MOUNT" => snap.disk_mount = v.to_string(),
            "LOAD" => snap.load = Some(v.to_string()),
            "UPTIME" => snap.uptime_secs = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    if !saw {
        bail!("empty host probe output");
    }
    if snap.hostname.is_empty() {
        snap.hostname = "—".into();
    }
    if snap.os.is_empty() {
        snap.os = "Unknown".into();
    }
    if snap.cpu_model.is_empty() {
        snap.cpu_model = "—".into();
    }
    if snap.disk_mount.is_empty() {
        snap.disk_mount = "/".into();
    }
    Ok(snap)
}

pub fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let n = n as f64;
    if n >= TB {
        format!("{:.1} TB", n / TB)
    } else if n >= GB {
        format!("{:.1} GB", n / GB)
    } else if n >= MB {
        format!("{:.0} MB", n / MB)
    } else if n >= KB {
        format!("{:.0} KB", n / KB)
    } else {
        format!("{n:.0} B")
    }
}

pub fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

/// Run the probe script on an already-connected russh session.
pub async fn collect_via_ssh(
    session: &russh::client::Handle<crate::session::ssh::ClientHandler>,
) -> Result<HostSnapshot> {
    use russh::ChannelMsg;
    use tokio::time::{Duration, timeout};

    let mut channel = session
        .channel_open_session()
        .await
        .context("open host-info channel")?;
    let script = remote_probe_script();
    // `exec` of a shell -c keeps the probe self-contained.
    let cmd = format!("sh -c {}", shell_single_quote(script));
    channel
        .exec(true, cmd)
        .await
        .context("exec host-info probe")?;

    let mut stdout = Vec::new();
    let read = async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { ref data }) => stdout.extend_from_slice(data),
                Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                    // Ignore stderr for parsing; still drain.
                    let _ = data;
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::ExitStatus { .. }) | None => break,
                _ => {}
            }
        }
    };
    timeout(Duration::from_secs(12), read)
        .await
        .context("host-info probe timed out")?;

    let text = String::from_utf8_lossy(&stdout);
    parse_remote_probe(&text).context("parse host-info probe")
}

fn shell_single_quote(s: &str) -> String {
    // Wrap in single quotes; escape embedded ' as: '\'' 
    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}
