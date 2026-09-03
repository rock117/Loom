//! Host snapshot for the Context panel Info tab (Local via sysinfo, SSH via remote probe).

use anyhow::{Context, Result, bail};

/// Max mounts shown in Info (primary + fullest others).
const MAX_DISKS: usize = 5;
/// Max GPUs shown in Info.
const MAX_GPUS: usize = 2;

/// One filesystem / mount usage row.
#[derive(Debug, Clone, Default)]
pub struct DiskUsage {
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

impl DiskUsage {
    pub fn ratio(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f64 / self.total as f64) as f32
        }
    }
}

/// One GPU row (best-effort; often name-only on non-NVIDIA).
#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub name: String,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    /// 0..=100 when the driver reports utilization.
    pub usage_pct: Option<f32>,
}

impl GpuInfo {
    pub fn vram_ratio(&self) -> Option<f32> {
        match (self.vram_used, self.vram_total) {
            (Some(used), Some(total)) if total > 0 => Some((used as f64 / total as f64) as f32),
            _ => None,
        }
    }
}

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
    /// Primary mount first, then other real mounts by fullness (capped).
    pub disks: Vec<DiskUsage>,
    /// Present only when detection succeeds (hidden in UI otherwise).
    pub gpus: Vec<GpuInfo>,
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

    let disks_sys = Disks::new_with_refreshed_list();
    let disks = collect_local_disks(&disks_sys);
    let gpus = collect_local_gpus();

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
        disks,
        gpus,
        load,
        uptime_secs: System::uptime(),
    })
}

fn collect_local_disks(disks: &sysinfo::Disks) -> Vec<DiskUsage> {
    let mut out: Vec<DiskUsage> = Vec::new();
    for disk in disks.list() {
        let total = disk.total_space();
        if total == 0 {
            continue;
        }
        let fs = disk.file_system().to_string_lossy().to_ascii_lowercase();
        if is_pseudo_fs(&fs) {
            continue;
        }
        let mount = disk.mount_point().to_string_lossy().into_owned();
        if mount.is_empty() {
            continue;
        }
        // Skip typical noise mounts.
        if is_noise_mount(&mount) {
            continue;
        }
        let avail = disk.available_space();
        let used = total.saturating_sub(avail);
        if out.iter().any(|d| mounts_equal(&d.mount, &mount)) {
            continue;
        }
        out.push(DiskUsage { mount, used, total });
    }
    rank_disks(out)
}

fn is_pseudo_fs(fs: &str) -> bool {
    const SKIP: &[&str] = &[
        "tmpfs",
        "devtmpfs",
        "devfs",
        "squashfs",
        "overlay",
        "overlay2",
        "aufs",
        "proc",
        "sysfs",
        "cgroup",
        "cgroup2",
        "rpc_pipefs",
        "fusectl",
        "tracefs",
        "debugfs",
        "securityfs",
        "pstore",
        "ramfs",
        "iso9660",
        "udf",
    ];
    SKIP.iter().any(|s| fs == *s)
}

fn is_noise_mount(mount: &str) -> bool {
    let m = mount.replace('\\', "/");
    let lower = m.to_ascii_lowercase();
    lower.starts_with("/snap/")
        || lower.starts_with("/var/lib/docker/")
        || lower.starts_with("/run/")
        || lower == "/boot/efi"
        || lower == "/boot/efi/"
        || lower.starts_with("/sys/")
        || lower.starts_with("/proc/")
        || lower.starts_with("/dev/")
}

fn mounts_equal(a: &str, b: &str) -> bool {
    let na = a.trim_end_matches(['/', '\\']);
    let nb = b.trim_end_matches(['/', '\\']);
    na.eq_ignore_ascii_case(nb)
}

fn is_primary_mount(mount: &str) -> bool {
    let m = mount.trim_end_matches(['/', '\\']);
    m.is_empty()
        || m == "/"
        || m.eq_ignore_ascii_case("C:")
        || m.eq_ignore_ascii_case("C:\\")
        || mount == "/"
        || mount.eq_ignore_ascii_case("C:\\")
        || mount.eq_ignore_ascii_case("C:/")
}

/// Primary first, then fullest; cap at [`MAX_DISKS`].
fn rank_disks(mut disks: Vec<DiskUsage>) -> Vec<DiskUsage> {
    if disks.is_empty() {
        return vec![DiskUsage {
            mount: "—".into(),
            used: 0,
            total: 0,
        }];
    }
    disks.sort_by(|a, b| {
        let pa = is_primary_mount(&a.mount);
        let pb = is_primary_mount(&b.mount);
        match (pa, pb) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b
                .ratio()
                .partial_cmp(&a.ratio())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.total.cmp(&a.total)),
        }
    });
    // Keep primary (if any) + next fullest unique; already sorted.
    disks.truncate(MAX_DISKS);
    disks
}

fn collect_local_gpus() -> Vec<GpuInfo> {
    if let Some(gpus) = probe_nvidia_smi() {
        return cap_gpus(gpus);
    }
    #[cfg(windows)]
    if let Some(gpus) = probe_windows_cim_gpus() {
        return cap_gpus(gpus);
    }
    #[cfg(not(windows))]
    if let Some(gpus) = probe_lspci_gpus() {
        return cap_gpus(gpus);
    }
    Vec::new()
}

fn cap_gpus(mut gpus: Vec<GpuInfo>) -> Vec<GpuInfo> {
    gpus.retain(|g| !g.name.trim().is_empty());
    gpus.truncate(MAX_GPUS);
    gpus
}

fn probe_nvidia_smi() -> Option<Vec<GpuInfo>> {
    let output = run_capture(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total,memory.used,utilization.gpu",
            "--format=csv,noheader,nounits",
        ],
    )?;
    let mut gpus = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.is_empty() || parts[0].is_empty() {
            continue;
        }
        let name = parts[0].to_string();
        let vram_total = parts
            .get(1)
            .and_then(|s| s.parse::<f64>().ok())
            .map(|mib| (mib * 1024.0 * 1024.0) as u64);
        let vram_used = parts
            .get(2)
            .and_then(|s| s.parse::<f64>().ok())
            .map(|mib| (mib * 1024.0 * 1024.0) as u64);
        let usage_pct = parts.get(3).and_then(|s| s.parse::<f32>().ok());
        gpus.push(GpuInfo {
            name,
            vram_used,
            vram_total,
            usage_pct,
        });
    }
    if gpus.is_empty() {
        None
    } else {
        Some(gpus)
    }
}

#[cfg(windows)]
fn probe_windows_cim_gpus() -> Option<Vec<GpuInfo>> {
    // Name|AdapterRAM — AdapterRAM is often wrong on modern GPUs; keep name, VRAM only if plausible.
    let script = "Get-CimInstance Win32_VideoController | ForEach-Object { '{0}|{1}' -f $_.Name, $_.AdapterRAM }";
    let output = run_capture(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    )?;
    let mut gpus: Vec<GpuInfo> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, ram) = match line.split_once('|') {
            Some((n, r)) => (n.trim(), r.trim()),
            None => (line, ""),
        };
        if name.is_empty() || is_dummy_gpu_name(name) {
            continue;
        }
        let vram_total = ram.parse::<u64>().ok().and_then(|b| {
            // Win32 AdapterRAM is a 32-bit field and often nonsense; only keep 256MB..=48GB.
            const MIN: u64 = 256 * 1024 * 1024;
            const MAX: u64 = 48u64 * 1024 * 1024 * 1024;
            if (MIN..=MAX).contains(&b) {
                Some(b)
            } else {
                None
            }
        });
        if gpus.iter().any(|g| g.name.eq_ignore_ascii_case(name)) {
            continue;
        }
        gpus.push(GpuInfo {
            name: name.to_string(),
            vram_used: None,
            vram_total,
            usage_pct: None,
        });
    }
    if gpus.is_empty() {
        None
    } else {
        Some(gpus)
    }
}

#[cfg(not(windows))]
fn probe_lspci_gpus() -> Option<Vec<GpuInfo>> {
    let output = run_capture("lspci", &[])?;
    let mut gpus = Vec::new();
    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        if !(lower.contains("vga compatible")
            || lower.contains("3d controller")
            || lower.contains("display controller"))
        {
            continue;
        }
        let name = line
            .split_once(": ")
            .map(|(_, rest)| rest.trim())
            .unwrap_or(line.trim());
        if name.is_empty() || is_dummy_gpu_name(name) {
            continue;
        }
        if gpus.iter().any(|g| g.name.eq_ignore_ascii_case(name)) {
            continue;
        }
        gpus.push(GpuInfo {
            name: name.to_string(),
            vram_used: None,
            vram_total: None,
            usage_pct: None,
        });
    }
    if gpus.is_empty() {
        None
    } else {
        Some(gpus)
    }
}

fn is_dummy_gpu_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("microsoft basic display")
        || n.contains("microsoft remote display")
        || n.contains("virtualbox")
        || (n.contains("aspeed") && n.contains("dummy"))
        || n == "unknown"
}

fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = crate::platform::new_command(program);
    cmd.args(args);
    let output = cmd.output().ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn parse_gpu_field(v: &str) -> GpuInfo {
    // GPU=name|vram_total_bytes|vram_used_bytes|usage_pct
    let mut parts = v.splitn(4, '|');
    let name = parts.next().unwrap_or("").trim().to_string();
    let vram_total = parts
        .next()
        .and_then(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                s.parse().ok()
            }
        });
    let vram_used = parts
        .next()
        .and_then(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                s.parse().ok()
            }
        });
    let usage_pct = parts.next().and_then(|s| {
        let s = s.trim();
        if s.is_empty() {
            None
        } else {
            s.parse().ok()
        }
    });
    GpuInfo {
        name,
        vram_used,
        vram_total,
        usage_pct,
    }
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
# Real mounts only; each DISK=mount|total|used (bytes).
df -B1 -P -x tmpfs -x devtmpfs -x squashfs -x overlay -x iso9660 -x udf 2>/dev/null \
  | awk 'NR>1 && $2+0>0 {printf "DISK=%s|%s|%s\n",$6,$2,$3}'
# GPU: prefer nvidia-smi; else lspci name-only.
if command -v nvidia-smi >/dev/null 2>&1; then
  nvidia-smi --query-gpu=name,memory.total,memory.used,utilization.gpu --format=csv,noheader,nounits 2>/dev/null \
    | awk -F',' '{
        gsub(/^ +| +$/,"",$1); gsub(/^ +| +$/,"",$2); gsub(/^ +| +$/,"",$3); gsub(/^ +| +$/,"",$4);
        if ($1!="") printf "GPU=%s|%.0f|%.0f|%s\n",$1,$2*1024*1024,$3*1024*1024,$4
      }'
elif command -v lspci >/dev/null 2>&1; then
  lspci 2>/dev/null | awk -F': ' '/VGA compatible|3D controller|Display controller/{printf "GPU=%s||||\n",$2}'
fi
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
            "DISK" => {
                let mut parts = v.splitn(3, '|');
                let mount = parts.next().unwrap_or("").trim();
                let total = parts
                    .next()
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
                let used = parts
                    .next()
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
                if mount.is_empty() || total == 0 || is_noise_mount(mount) {
                    continue;
                }
                if snap.disks.iter().any(|d| mounts_equal(&d.mount, mount)) {
                    continue;
                }
                snap.disks.push(DiskUsage {
                    mount: mount.to_string(),
                    used,
                    total,
                });
            }
            // Legacy single-disk keys (older probe / fallback).
            "DISK_TOTAL" => {
                if let Some(d) = snap.disks.first_mut() {
                    d.total = v.parse().unwrap_or(d.total);
                } else {
                    snap.disks.push(DiskUsage {
                        mount: "/".into(),
                        used: 0,
                        total: v.parse().unwrap_or(0),
                    });
                }
            }
            "DISK_USED" => {
                if let Some(d) = snap.disks.first_mut() {
                    d.used = v.parse().unwrap_or(d.used);
                } else {
                    snap.disks.push(DiskUsage {
                        mount: "/".into(),
                        used: v.parse().unwrap_or(0),
                        total: 0,
                    });
                }
            }
            "DISK_MOUNT" => {
                if let Some(d) = snap.disks.first_mut() {
                    d.mount = v.to_string();
                } else {
                    snap.disks.push(DiskUsage {
                        mount: v.to_string(),
                        used: 0,
                        total: 0,
                    });
                }
            }
            "LOAD" => snap.load = Some(v.to_string()),
            "UPTIME" => snap.uptime_secs = v.parse().unwrap_or(0),
            "GPU" => {
                let gpu = parse_gpu_field(v);
                if gpu.name.is_empty() || is_dummy_gpu_name(&gpu.name) {
                    continue;
                }
                if snap
                    .gpus
                    .iter()
                    .any(|g| g.name.eq_ignore_ascii_case(&gpu.name))
                {
                    continue;
                }
                snap.gpus.push(gpu);
            }
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
    snap.disks = rank_disks(std::mem::take(&mut snap.disks));
    snap.gpus = cap_gpus(std::mem::take(&mut snap.gpus));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_primary_then_fullest() {
        let ranked = rank_disks(vec![
            DiskUsage {
                mount: "/data".into(),
                used: 90,
                total: 100,
            },
            DiskUsage {
                mount: "/".into(),
                used: 10,
                total: 100,
            },
            DiskUsage {
                mount: "/home".into(),
                used: 50,
                total: 100,
            },
        ]);
        assert_eq!(ranked[0].mount, "/");
        assert_eq!(ranked[1].mount, "/data");
        assert_eq!(ranked[2].mount, "/home");
    }

    #[test]
    fn parses_multi_disk_probe() {
        let snap = parse_remote_probe(
            "HOSTNAME=box\nOS=Linux\nDISK=/|100|40\nDISK=/data|200|180\nDISK=/home|100|20\n",
        )
        .unwrap();
        assert_eq!(snap.disks.len(), 3);
        assert_eq!(snap.disks[0].mount, "/");
        assert_eq!(snap.disks[1].mount, "/data");
    }
}
