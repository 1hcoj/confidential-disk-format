//! Resolve a disk file or block device, optionally attaching a loop device.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug)]
pub struct DiskHandle {
    /// Block device used for GPT / partitions (loop or real).
    pub block_dev: PathBuf,
    /// True if this process attached the loop device.
    pub owns_loop: bool,
}

impl DiskHandle {
    pub fn open(path: &Path, attach_loop: bool) -> Result<Self> {
        if !path.exists() {
            bail!("{} does not exist", path.display());
        }
        let meta = std::fs::metadata(path)?;
        let is_block = is_block_device(path)?;

        if is_block {
            return Ok(Self {
                block_dev: path.to_path_buf(),
                owns_loop: false,
            });
        }

        if !meta.is_file() {
            bail!(
                "{} is neither a regular file nor a block device",
                path.display()
            );
        }

        if !attach_loop {
            return Ok(Self {
                block_dev: path.to_path_buf(),
                owns_loop: false,
            });
        }

        let loop_dev = losetup_attach(path)?;
        Ok(Self {
            block_dev: PathBuf::from(loop_dev),
            owns_loop: true,
        })
    }

    pub fn detach_loop(&self) -> Result<()> {
        if self.owns_loop {
            // Ensure partition writes hit the backing file before detach.
            let _ = Command::new("blockdev")
                .args(["--flushbufs"])
                .arg(&self.block_dev)
                .status();
            let _ = Command::new("sync").status();
            run_checked(&mut Command::new("losetup").arg("-d").arg(&self.block_dev))?;
        }
        Ok(())
    }
}

pub fn is_block_device(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        Ok(std::fs::metadata(path)?.file_type().is_block_device())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

pub fn losetup_attach(file: &Path) -> Result<String> {
    // --partscan so partition nodes (loopXpN) appear after GPT write.
    let out = Command::new("losetup")
        .args(["--find", "--show", "--partscan"])
        .arg(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("run losetup")?;
    if !out.status.success() {
        bail!(
            "losetup failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let dev = String::from_utf8(out.stdout)?.trim().to_string();
    if dev.is_empty() {
        bail!("losetup returned empty device path");
    }
    // Give udev / kernel a moment to create partition nodes after later GPT ops.
    Ok(dev)
}

pub fn partprobe(dev: &Path) -> Result<()> {
    // Best-effort kernel partition refresh. On a blank image (no GPT yet) partx/
    // partprobe often print "failed to read partition table" — that is expected
    // during create's pre-check, so stderr is discarded.
    if which("partprobe") {
        let _ = Command::new("partprobe")
            .arg(dev)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if which("partx") {
        let _ = Command::new("partx")
            .args(["-u"])
            .arg(dev)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = Command::new("losetup")
        .args(["-c"])
        .arg(dev)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // Brief settle for /dev/loopXpN
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(())
}

pub fn which(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn run_checked(cmd: &mut Command) -> Result<()> {
    let prog = format!("{cmd:?}");
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("spawn {prog}"))?;
    if !out.status.success() {
        bail!(
            "command failed: {prog}\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

pub fn run_output(cmd: &mut Command) -> Result<String> {
    let prog = format!("{cmd:?}");
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("spawn {prog}"))?;
    if !out.status.success() {
        bail!(
            "command failed: {prog}\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

pub fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim().to_uppercase();
    let (num, mult) = if let Some(n) = s.strip_suffix('G') {
        (n, 1u64 << 30)
    } else if let Some(n) = s.strip_suffix('M') {
        (n, 1u64 << 20)
    } else if let Some(n) = s.strip_suffix('K') {
        (n, 1u64 << 10)
    } else if let Some(n) = s.strip_suffix('B') {
        (n, 1)
    } else {
        (s.as_str(), 1)
    };
    let n: u64 = num.parse().with_context(|| format!("parse size {s}"))?;
    Ok(n.checked_mul(mult).context("size overflow")?)
}

pub fn ensure_image_file(path: &Path, size: u64) -> Result<()> {
    if path.exists() {
        let meta = std::fs::metadata(path)?;
        if meta.len() < size {
            bail!(
                "{} exists but is smaller than requested size ({} < {})",
                path.display(),
                meta.len(),
                size
            );
        }
        return Ok(());
    }
    let f = std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    f.set_len(size)?;
    Ok(())
}

/// Partition node for GPT partition index N (1-based).
pub fn partition_path(block_dev: &Path, index: u32) -> Result<PathBuf> {
    let s = block_dev.to_string_lossy();
    let candidate = if s
        .chars()
        .last()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        // e.g. /dev/loop0 → /dev/loop0p1 ; /dev/nvme0n1 → /dev/nvme0n1p1
        PathBuf::from(format!("{s}p{index}"))
    } else {
        // e.g. /dev/sda → /dev/sda1
        PathBuf::from(format!("{s}{index}"))
    };
    if candidate.exists() {
        return Ok(candidate);
    }
    // Wait a bit and retry (partition scan race).
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!(
        "partition device {} did not appear; try partprobe/udevadm settle",
        candidate.display()
    );
}

pub fn device_size_bytes(path: &Path) -> Result<u64> {
    if is_block_device(path)? {
        // Prefer blockdev --getsize64
        if which("blockdev") {
            let out = run_output(&mut Command::new("blockdev").arg("--getsize64").arg(path))?;
            return Ok(out.trim().parse()?);
        }
    }
    Ok(std::fs::metadata(path)?.len())
}
