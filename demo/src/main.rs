mod ops;

use anyhow::Result;
use cdf::Profile;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "cdf-gpt",
    about = "CDF v1 GPT demo tool: info / create / open",
    long_about = "Stateless helper for the GPT-based Confidential Disk Format (see design/README.md).\n\
\n\
State machine:\n  S0 (blank) --create--> S1 (sealed CDF) --open--> S2 (mapped)\n\
Non-blank disks are only re-created with --force.\n\
Pass a disk image file (losetup) or a block device path."
)]
struct Cli {
    /// Disk image file or block device (e.g. ./disk.img or /dev/sdb).
    disk: PathBuf,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Show GPT layout, capacity, and CDF metadata (optionally verify MAC).
    Info {
        /// K_root as hex (64 nibbles) or arbitrary string (SHA-256'd to 32 bytes).
        #[arg(long)]
        key: Option<String>,
        /// K_root from file (32 raw bytes, 64 hex chars, or hashed contents).
        #[arg(long)]
        key_file: Option<String>,
    },
    /// Initialize a CDF volume (GPT + seal). Refuses non-blank disks unless --force.
    Create {
        /// plain | crypt+integrity | verity | verity+crypt
        #[arg(long)]
        profile: String,
        /// Image size when creating a new file (e.g. 256M, 1G). Forbidden for block devices.
        #[arg(long)]
        size: Option<String>,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        key_file: Option<String>,
        /// Re-format even if the disk is not blank (existing CDF or other content).
        #[arg(long)]
        force: bool,
        /// Final payload image for verity / verity+crypt (copied into P_data before seal).
        #[arg(long)]
        from_image: Option<PathBuf>,
        /// Optional Contract.key_locator hint (e.g. kbs:///default/cdf/orders-db).
        /// Deployment-supplied locators always override this field.
        #[arg(long)]
        key_locator: Option<String>,
        /// Create an ext4 filesystem on the data device before sealing
        /// (plain / crypt+integrity). Volumes are never formatted on open.
        #[arg(long)]
        mkfs: bool,
    },
    /// Open a sealed CDF volume (verify MAC, map to dm). Never formats.
    Open {
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        key_file: Option<String>,
        /// dm-crypt / dm-verity mapper name (default: cdf-data).
        #[arg(long, default_value = "cdf-data")]
        mapper: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Info { key, key_file } => ops::cmd_info(ops::InfoArgs {
            disk: cli.disk,
            key,
            key_file,
        }),
        Cmd::Create {
            profile,
            size,
            key,
            key_file,
            force,
            from_image,
            key_locator,
            mkfs,
        } => ops::cmd_create(ops::CreateArgs {
            disk: cli.disk,
            profile: Profile::parse(&profile)?,
            size,
            key,
            key_file,
            force,
            from_image,
            key_locator,
            mkfs,
        }),
        Cmd::Open {
            key,
            key_file,
            mapper,
        } => ops::cmd_open(ops::OpenArgs {
            disk: cli.disk,
            key,
            key_file,
            mapper,
        }),
    }
}
