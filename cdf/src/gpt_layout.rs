//! GPT layout helpers via `sgdisk` (design §4.1).

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

use crate::device::{partprobe, run_checked, run_output};
use crate::format::{
    Profile, LABEL_DATA, LABEL_META, LABEL_VERITY, META_MIB, TYPE_DATA, TYPE_META, TYPE_VERITY,
    VERITY_MIB,
};

#[derive(Debug, Clone)]
pub struct Layout {
    pub has_verity: bool,
    /// 1-based indices matching sgdisk creation order.
    pub meta_index: u32,
    pub verity_index: Option<u32>,
    pub data_index: u32,
}

impl Layout {
    pub fn for_profile(profile: Profile) -> Self {
        if profile.needs_verity() {
            Self {
                has_verity: true,
                meta_index: 1,
                verity_index: Some(2),
                data_index: 3,
            }
        } else {
            Self {
                has_verity: false,
                meta_index: 1,
                verity_index: None,
                data_index: 2,
            }
        }
    }
}

/// Wipe and create a fresh CDF GPT on `block_dev`.
pub fn create_partitions(block_dev: &Path, profile: Profile) -> Result<Layout> {
    let layout = Layout::for_profile(profile);

    // Clear any existing table.
    run_checked(&mut Command::new("sgdisk").args(["--zap-all"]).arg(block_dev))?;

    // Partition sizes as sgdisk +N M endings (MiB).
    let meta = format!("+{META_MIB}M");
    let mut cmd = Command::new("sgdisk");
    cmd.arg("-n").arg(format!("1:0:{meta}"));
    cmd.arg("-t").arg(format!("1:{TYPE_META}"));
    cmd.arg("-c").arg(format!("1:{LABEL_META}"));

    if layout.has_verity {
        let verity = format!("+{VERITY_MIB}M");
        cmd.arg("-n").arg(format!("2:0:{verity}"));
        cmd.arg("-t").arg(format!("2:{TYPE_VERITY}"));
        cmd.arg("-c").arg(format!("2:{LABEL_VERITY}"));

        cmd.arg("-n").arg("3:0:0"); // rest
        cmd.arg("-t").arg(format!("3:{TYPE_DATA}"));
        cmd.arg("-c").arg(format!("3:{LABEL_DATA}"));
    } else {
        cmd.arg("-n").arg("2:0:0");
        cmd.arg("-t").arg(format!("2:{TYPE_DATA}"));
        cmd.arg("-c").arg(format!("2:{LABEL_DATA}"));
    }

    cmd.arg(block_dev);
    run_checked(&mut cmd)?;
    partprobe(block_dev)?;
    Ok(layout)
}

#[derive(Debug, Clone)]
pub struct Discovered {
    pub layout: Layout,
    pub meta: std::path::PathBuf,
    pub verity: Option<std::path::PathBuf>,
    pub data: std::path::PathBuf,
}

/// Discover CDF partitions by type UUID using `sgdisk -i`.
pub fn discover(block_dev: &Path) -> Result<Discovered> {
    // Ensure partition nodes exist.
    partprobe(block_dev)?;

    let print = run_output(&mut Command::new("sgdisk").args(["-p"]).arg(block_dev))?;
    let _ = print;

    let mut meta_idx = None;
    let mut verity_idx = None;
    let mut data_idx = None;

    // Probe partitions 1..8
    for idx in 1u32..=8 {
        let info = match run_output(
            &mut Command::new("sgdisk")
                .args(["-i", &idx.to_string()])
                .arg(block_dev),
        ) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if info.contains("does not exist") || info.contains("Partition GUID code: 00000000") {
            continue;
        }
        // sgdisk prints "Partition GUID code: XXXX-... (type name)" — type UUID first.
        let type_uuid = extract_type_guid(&info).unwrap_or_default().to_lowercase();
        if type_uuid == TYPE_META {
            meta_idx = Some(idx);
        } else if type_uuid == TYPE_VERITY {
            verity_idx = Some(idx);
        } else if type_uuid == TYPE_DATA {
            data_idx = Some(idx);
        }
    }

    let meta_index = meta_idx.context("CDF meta partition not found (type UUID)")?;
    let data_index = data_idx.context("CDF data partition not found (type UUID)")?;
    let layout = Layout {
        has_verity: verity_idx.is_some(),
        meta_index,
        verity_index: verity_idx,
        data_index,
    };

    let meta = crate::device::partition_path(block_dev, meta_index)?;
    let data = crate::device::partition_path(block_dev, data_index)?;
    let verity = match verity_idx {
        Some(i) => Some(crate::device::partition_path(block_dev, i)?),
        None => None,
    };

    Ok(Discovered {
        layout,
        meta,
        verity,
        data,
    })
}

fn extract_type_guid(info: &str) -> Option<String> {
    // Lines look like:
    // Partition GUID code: 4CA88EE1-60F4-4B6B-899E-DAE026CED8E8 (CDF meta)
    for line in info.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Partition GUID code:") {
            let token = rest.trim().split_whitespace().next()?;
            return Some(token.to_lowercase());
        }
    }
    None
}

pub fn require_root() -> Result<()> {
    #[cfg(unix)]
    {
        if libc_geteuid() != 0 {
            bail!("this operation needs root (losetup / device-mapper / cryptsetup)");
        }
    }
    Ok(())
}

#[cfg(unix)]
fn libc_geteuid() -> u32 {
    // Avoid a libc crate dependency: read /proc/self/status
    if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("Uid:") {
                if let Some(uid) = v.split_whitespace().next() {
                    if let Ok(n) = uid.parse() {
                        return n;
                    }
                }
            }
        }
    }
    // Fallback: assume non-root if unknown.
    1
}
