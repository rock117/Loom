//! Local zip + remote archive helpers for compressed transfers.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use russh::ChannelMsg;
use russh::client;
use tokio::time::{Duration, timeout};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::ssh::ClientHandler;
use super::sftp::TransferCancel;
use super::transfer_filter::FilterMatcher;
use std::sync::atomic::Ordering;

fn ensure_not_cancelled(cancel: &TransferCancel) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    Ok(())
}

fn shell_single_quote(s: &str) -> String {
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

/// Build a local `.zip` of `src` (file or directory), applying `filter` to relative paths.
pub fn zip_local(src: &Path, dest_zip: &Path, filter: &FilterMatcher, cancel: &TransferCancel) -> Result<u64> {
    ensure_not_cancelled(cancel)?;
    if let Some(parent) = dest_zip.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let file = File::create(dest_zip).with_context(|| format!("create {}", dest_zip.display()))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let root_name = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".into());

    if src.is_file() {
        if filter.allows(&root_name, false) {
            zip.start_file(root_name.replace('\\', "/"), opts)
                .context("zip start_file")?;
            let mut f = File::open(src).with_context(|| format!("open {}", src.display()))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            zip.write_all(&buf)?;
        }
    } else if src.is_dir() {
        add_dir_to_zip(&mut zip, src, &root_name, filter, opts, cancel)?;
    } else {
        bail!("unsupported path for zip: {}", src.display());
    }
    let mut finished = zip.finish().context("zip finish")?;
    finished.flush().context("zip flush")?;
    drop(finished);

    let meta = std::fs::metadata(dest_zip).with_context(|| format!("stat {}", dest_zip.display()))?;
    if meta.len() == 0 {
        bail!("zip archive is empty ({})", dest_zip.display());
    }
    Ok(meta.len())
}

fn add_dir_to_zip(
    zip: &mut ZipWriter<File>,
    dir: &Path,
    prefix: &str,
    filter: &FilterMatcher,
    opts: SimpleFileOptions,
    cancel: &TransferCancel,
) -> Result<()> {
    ensure_not_cancelled(cancel)?;
    if !filter.allows(prefix, true) {
        return Ok(());
    }
    // Directory entry
    let dir_name = format!("{prefix}/");
    let _ = zip.add_directory(&dir_name, opts);

    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        ensure_not_cancelled(cancel)?;
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let rel = format!("{prefix}/{name}");
        let path = entry.path();
        let is_dir = path.is_dir();
        if !filter.allows(&rel, is_dir) {
            continue;
        }
        if is_dir {
            add_dir_to_zip(zip, &path, &rel, filter, opts, cancel)?;
        } else {
            zip.start_file(&rel, opts)?;
            let mut f = File::open(&path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            zip.write_all(&buf)?;
        }
    }
    Ok(())
}

/// Extract a local archive (zip / tar / tar.gz / tar.bz2 / tgz / gz) into `dest_dir`.
pub fn extract_local(archive: &Path, dest_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dest_dir)?;
    let name = archive
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if name.ends_with(".zip") {
        extract_zip(archive, dest_dir)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, dest_dir)
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        extract_tar_bz2(archive, dest_dir)
    } else if name.ends_with(".tar") {
        extract_tar(archive, dest_dir)
    } else if name.ends_with(".gz") && !name.ends_with(".tar.gz") {
        extract_gzip_file(archive, dest_dir)
    } else {
        // Leave as-is: copy into dest
        let dest = dest_dir.join(archive.file_name().unwrap_or_default());
        std::fs::copy(archive, &dest)?;
        Ok(())
    }
}

fn extract_zip(archive: &Path, dest_dir: &Path) -> Result<()> {
    let f = File::open(archive)?;
    let mut zip = ZipArchive::new(f).context("open zip")?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(p) => dest_dir.join(p),
            None => continue,
        };
        if file.name().ends_with('/') {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}

fn extract_tar(archive: &Path, dest_dir: &Path) -> Result<()> {
    let f = File::open(archive)?;
    let mut archive = tar::Archive::new(f);
    archive.unpack(dest_dir).context("untar")?;
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest_dir: &Path) -> Result<()> {
    let f = File::open(archive)?;
    let dec = flate2::read::GzDecoder::new(f);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(dest_dir).context("untar.gz")?;
    Ok(())
}

fn extract_tar_bz2(archive: &Path, dest_dir: &Path) -> Result<()> {
    // Prefer system `tar` when available (bzip2 feature not always linked).
    let status = Command::new("tar")
        .args(["-xjf"])
        .arg(archive)
        .arg("-C")
        .arg(dest_dir)
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => bail!("cannot extract .tar.bz2 (need `tar` on PATH)"),
    }
}

fn extract_gzip_file(archive: &Path, dest_dir: &Path) -> Result<()> {
    let f = File::open(archive)?;
    let mut dec = flate2::read::GzDecoder::new(f);
    let stem = archive
        .file_stem()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| "file".into());
    let out = dest_dir.join(stem);
    let mut outfile = File::create(&out)?;
    std::io::copy(&mut dec, &mut outfile)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct RemoteArchive {
    pub remote_path: String,
    pub ext: &'static str,
}

/// Try to create a remote archive of `remote` (file or dir) under `/tmp`.
/// Tries zip → tar.gz → tar → 7z → gzip (files). Applies simple name excludes via tool flags.
pub async fn remote_create_archive(
    session: &client::Handle<ClientHandler>,
    remote: &str,
    excludes: &[String],
    cancel: &TransferCancel,
) -> Result<RemoteArchive> {
    ensure_not_cancelled(cancel)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let parent = remote_parent(remote);
    let base = remote_basename(remote);
    let parent_q = shell_single_quote(&parent);
    let base_q = shell_single_quote(&base);

    let exclude_zip: String = excludes
        .iter()
        .filter(|p| is_tool_friendly(p))
        .map(|p| format!(" -x {}", shell_single_quote(&format!("*/{p}/*"))))
        .collect();
    let exclude_tar: String = excludes
        .iter()
        .filter(|p| is_tool_friendly(p))
        .map(|p| format!(" --exclude={}", shell_single_quote(p)))
        .collect();

    // (ext, command that creates /tmp/loom-xfer-ID.EXT and exits 0)
    let attempts: Vec<(&str, String)> = vec![
        (
            "zip",
            format!(
                "cd {parent_q} && zip -r /tmp/loom-xfer-{id}.zip {base_q}{exclude_zip} >/dev/null"
            ),
        ),
        (
            "tar.gz",
            format!(
                "cd {parent_q} && tar -czf /tmp/loom-xfer-{id}.tar.gz{exclude_tar} {base_q}"
            ),
        ),
        (
            "tar",
            format!(
                "cd {parent_q} && tar -cf /tmp/loom-xfer-{id}.tar{exclude_tar} {base_q}"
            ),
        ),
        (
            "7z",
            format!(
                "cd {parent_q} && 7z a -tzip /tmp/loom-xfer-{id}.7z {base_q} >/dev/null"
            ),
        ),
        // Single-file gzip fallback (works when remote is a file).
        (
            "gz",
            {
                let remote_q = shell_single_quote(remote);
                format!("gzip -c -- {remote_q} > /tmp/loom-xfer-{id}.gz")
            },
        ),
    ];

    let mut last_err = String::new();
    for (ext, cmd) in attempts {
        ensure_not_cancelled(cancel)?;
        match remote_exec_ok(session, &cmd).await {
            Ok(()) => {
                let remote_path = format!("/tmp/loom-xfer-{id}.{ext}");
                // Verify the archive exists via a cheap test.
                let test = format!("test -s {}", shell_single_quote(&remote_path));
                if remote_exec_ok(session, &test).await.is_ok() {
                    return Ok(RemoteArchive { remote_path, ext });
                }
                last_err = format!("{ext}: archive missing after command");
            }
            Err(e) => {
                last_err = format!("{ext}: {e:#}");
            }
        }
    }
    bail!("no remote archive tool worked ({last_err})")
}

fn is_tool_friendly(pat: &str) -> bool {
    !pat.is_empty()
        && !pat.contains('*')
        && !pat.contains('?')
        && !pat.contains('[')
        && !pat.contains('/')
        && !pat.contains('\\')
}

fn remote_parent(path: &str) -> String {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => path[..i].to_string(),
        None => ".".into(),
    }
}

fn remote_basename(path: &str) -> String {
    let path = path.trim_end_matches('/');
    path.rsplit('/').next().unwrap_or(path).to_string()
}

async fn remote_exec_ok(session: &client::Handle<ClientHandler>, cmd: &str) -> Result<()> {
    let mut channel = session
        .channel_open_session()
        .await
        .context("open exec channel")?;
    let wrapped = format!("sh -c {}", shell_single_quote(cmd));
    channel.exec(true, wrapped).await.context("exec")?;

    let mut stderr = Vec::new();
    let mut status = None;
    let read = async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { .. }) => {}
                Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                    stderr.extend_from_slice(data);
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    status = Some(exit_status);
                    // Prefer exit status; channel may still send Eof after.
                    break;
                }
                Some(ChannelMsg::Eof) => {
                    // Wait for ExitStatus if it hasn't arrived yet.
                    if status.is_some() {
                        break;
                    }
                }
                None => break,
                _ => {}
            }
        }
    };
    timeout(Duration::from_secs(600), read)
        .await
        .context("remote archive timed out")?;

    match status {
        Some(0) => Ok(()),
        Some(code) => {
            let err = String::from_utf8_lossy(&stderr);
            bail!("exit {code}: {err}");
        }
        None => bail!("no exit status: {}", String::from_utf8_lossy(&stderr)),
    }
}

pub fn temp_zip_path(label: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push("loom-xfer");
    let _ = std::fs::create_dir_all(&p);
    let safe: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    p.push(format!(
        "{}-{}.zip",
        safe,
        uuid::Uuid::new_v4().simple()
    ));
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::sftp::transfer_cancel_flag;
    use crate::session::transfer_filter::TransferFilter;

    #[test]
    fn zip_local_packs_directory() {
        let dir = std::env::temp_dir().join(format!("loom-zip-src-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), b"world").unwrap();
        // Default excludes should not strip these files.
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("node_modules").join("x.js"), b"skip").unwrap();

        let out = std::env::temp_dir().join(format!("loom-zip-out-{}.zip", uuid::Uuid::new_v4()));
        let m = TransferFilter::with_default_excludes().matcher().unwrap();
        let cancel = transfer_cancel_flag();
        let n = zip_local(&dir, &out, &m, &cancel).expect("zip_local");
        assert!(n > 0, "archive size");
        assert!(out.is_file());

        let f = File::open(&out).unwrap();
        let mut z = ZipArchive::new(f).unwrap();
        let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
        assert!(names.iter().any(|n| n.ends_with("a.txt")), "{names:?}");
        assert!(
            !names.iter().any(|n| n.contains("node_modules")),
            "node_modules should be excluded: {names:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&out);
    }
}
