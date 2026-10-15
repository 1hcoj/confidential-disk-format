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

/// Best-effort kernel partition-table refresh.
///
/// Tools differ by environment (host has `partprobe`; minimal guest images
/// often only ship util-linux `partx` / `blockdev`), so every helper is tried
/// and missing binaries are ignored. On a blank disk these print "failed to
/// read partition table", which is expected, so output is discarded.
pub fn partprobe(dev: &Path) -> Result<()> {
    let attempts: [&[&str]; 4] = [
        &["partprobe"],
        &["partx", "-u"],
        &["blockdev", "--rereadpt"],
        &["losetup", "-c"],
    ];
    for argv in attempts {
        let _ = Command::new(argv[0])
            .args(&argv[1..])
            .arg(dev)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(())
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
    use std::io::{Seek, SeekFrom};
    if is_block_device(path)? {
        let mut f =
            std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        return Ok(f.seek(SeekFrom::End(0))?);
    }
    Ok(std::fs::metadata(path)?.len())
}

/// True if every byte of `[offset, offset + len)` (clamped to the device) is zero.
pub fn range_is_zero(path: &Path, offset: u64, len: u64) -> Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    let size = device_size_bytes(path)?;
    if offset >= size {
        return Ok(true);
    }
    let mut left = len.min(size - offset);
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; 1 << 20];
    while left > 0 {
        let n = (left as usize).min(buf.len());
        f.read_exact(&mut buf[..n])
            .with_context(|| format!("read {}", path.display()))?;
        if buf[..n].iter().any(|b| *b != 0) {
            return Ok(false);
        }
        left -= n as u64;
    }
    Ok(true)
}
