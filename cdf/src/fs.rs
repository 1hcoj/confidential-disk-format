//! Filesystem creation on a freshly formatted data device.

use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::device::{run_checked, run_output};

/// `mkfs.ext4` that also works on un-wiped dm-integrity.
///
/// Sectors that were never written carry no valid tag and fail to read, and
/// mke2fs reads some blocks before writing them. With `integrity`, every
/// superblock location is first written whole (via a dry run that lists them),
/// and inode tables / journal are initialized eagerly so nothing is left for
/// the kernel to read lazily later.
/// See https://gitlab.com/cryptsetup/cryptsetup/-/issues/525.
pub fn mkfs_ext4(dev: &Path, integrity: bool) -> Result<()> {
    if integrity {
        let dry = run_output(
            Command::new("mkfs.ext4")
                .args(["-F", "-n", "-b", "4096"])
                .arg(dev),
        )?;
        let mut blocks = vec![0u64];
        if let Some((_, tail)) = dry.split_once("Superblock backups stored on blocks:") {
            blocks.extend(
                tail.split(|c: char| c == ',' || c.is_whitespace())
                    .filter_map(|t| t.trim().parse::<u64>().ok()),
            );
        }
        for b in blocks {
            run_checked(Command::new("dd").args([
                "if=/dev/zero",
                &format!("of={}", dev.display()),
                "bs=4096",
                "count=1",
                "oflag=direct",
                &format!("seek={b}"),
                "status=none",
            ]))?;
        }
    }
    run_checked(
        Command::new("mkfs.ext4")
            .args([
                "-F",
                "-q",
                "-b",
                "4096",
                "-E",
                "lazy_itable_init=0,lazy_journal_init=0",
            ])
            .arg(dev),
    )
}
