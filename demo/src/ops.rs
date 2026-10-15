//! High-level create / info / open operations.
//!
//! State machine:
//!   S0 (blank) --create--> S1 (sealed CDF GPT)
//!   S1 --open--> S2 (mapped; teardown commands printed; disk unchanged)
//! Non-blank disks (sealed or foreign) are only re-created with --force.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

use cdf::device::{device_size_bytes, ensure_image_file, is_block_device, parse_size, DiskHandle};
use cdf::format::{read_meta, verify_mac, Profile, META_MIB, VERITY_MIB};
use cdf::gpt_layout;
use cdf::keys::ensure_key_material;
use cdf::volume::Teardown;
use cdf::{CreateRequest, DiskState};

pub struct CreateArgs {
    pub disk: PathBuf,
    pub profile: Profile,
    pub size: Option<String>,
    pub key: Option<String>,
    pub key_file: Option<String>,
    pub force: bool,
    pub from_image: Option<PathBuf>,
    /// Optional on-disk Contract.key_locator hint (e.g. kbs:///…).
    pub key_locator: Option<String>,
    pub mkfs: bool,
}

pub struct InfoArgs {
    pub disk: PathBuf,
    pub key: Option<String>,
    pub key_file: Option<String>,
}

pub struct OpenArgs {
    pub disk: PathBuf,
    pub key: Option<String>,
    pub key_file: Option<String>,
    pub mapper: String,
}

pub fn cmd_create(args: CreateArgs) -> Result<()> {
    gpt_layout::require_root()?;
    let root = ensure_key_material(args.key.as_deref(), args.key_file.as_deref())?;
    if args.mkfs && args.profile.needs_verity() {
        bail!("--mkfs is only valid for plain / crypt+integrity");
    }

    let is_block = is_block_device(&args.disk).unwrap_or(false);
    let size_bytes = match &args.size {
        Some(s) => {
            if is_block {
                bail!("--size is not used for block devices; capacity comes from the device");
            }
            parse_size(s)?
        }
        None => {
            if !args.disk.exists() {
                bail!("--size is required when creating a new image file");
            }
            device_size_bytes(&args.disk)?
        }
    };

    let min = (META_MIB
        + if args.profile.needs_verity() {
            VERITY_MIB
        } else {
            0
        }
        + 64)
        << 20;
    if size_bytes < min {
        bail!(
            "image too small ({size_bytes} B); need at least {min} B for profile {}",
            args.profile
        );
    }

    if !is_block {
        ensure_image_file(&args.disk, size_bytes)?;
    }

    let handle = DiskHandle::open(&args.disk, true)?;
    let result = (|| {
        if args.force {
            match cdf::probe(&handle.block_dev)? {
                DiskState::Blank => {}
                DiskState::Sealed(_) => eprintln!(
                    "warning: --force: destroying existing CDF layout on {}",
                    args.disk.display()
                ),
                DiskState::Foreign(_) => eprintln!(
                    "warning: --force: overwriting non-CDF content on {}",
                    args.disk.display()
                ),
            }
        }

        let integrity = args.profile.needs_luks();
        let mkfs = |dev: &std::path::Path| cdf::fs::mkfs_ext4(dev, integrity);
        let created = cdf::create(CreateRequest {
            block_dev: &handle.block_dev,
            profile: args.profile,
            root: &root,
            key_locator: args.key_locator.clone(),
            from_image: args.from_image.as_deref(),
            populate: if args.mkfs { Some(&mkfs) } else { None },
            force: args.force,
        })
        .map_err(|e| {
            if args.force {
                e
            } else {
                e.context("pass --force to re-format")
            }
        })?;
        let disc = &created.discovered;
        eprintln!(
            "sealed volume_uuid={} profile={} generation=1",
            created.volume_uuid, args.profile
        );
        println!("created CDF image");
        println!("  disk:          {}", args.disk.display());
        println!("  block:         {}", handle.block_dev.display());
        println!("  profile:       {}", args.profile);
        println!("  access_mode:   {}", args.profile.access_mode());
        println!("  volume_uuid:   {}", created.volume_uuid);
        println!("  filesystem:    {}", if args.mkfs { "ext4" } else { "(none)" });
        if let Some(loc) = &args.key_locator {
            println!("  key_locator:   {loc}");
        }
        println!("  meta:          {}", disc.meta.display());
        if let Some(v) = &disc.verity {
            println!("  verity:        {}", v.display());
        }
        println!("  data:          {}", disc.data.display());
        println!(
            "  layout:        meta=#{} data=#{}",
            disc.layout.meta_index, disc.layout.data_index
        );
        Ok(())
    })();

    let _ = handle.detach_loop();
    result
}

pub fn cmd_info(args: InfoArgs) -> Result<()> {
    let need_loop = !is_block_device(&args.disk).unwrap_or(false);
    if need_loop {
        gpt_layout::require_root()?;
    }
    let handle = DiskHandle::open(&args.disk, need_loop)?;
    let result = (|| {
        let size = device_size_bytes(&handle.block_dev)?;
        println!("disk:            {}", args.disk.display());
        println!("block_device:     {}", handle.block_dev.display());
        println!("capacity_bytes:  {size}");
        println!("capacity_human:  {}", human_bytes(size));

        let disc = match cdf::probe(&handle.block_dev)? {
            DiskState::Blank => {
                println!("cdf:             blank (no sealed CDF metadata)");
                return Ok(());
            }
            DiskState::Foreign(why) => {
                println!("cdf:             foreign content, not CDF ({why})");
                return Ok(());
            }
            DiskState::Sealed(d) => d,
        };

        println!("partitions:");
        println!(
            "  meta:          {} (#{})",
            disc.meta.display(),
            disc.layout.meta_index
        );
        if let Some(v) = &disc.verity {
            println!(
                "  verity:        {} (#{})",
                v.display(),
                disc.layout.verity_index.unwrap()
            );
        }
        println!(
            "  data:          {} (#{})",
            disc.data.display(),
            disc.layout.data_index
        );
        println!(
            "  meta_size:     {}",
            human_bytes(device_size_bytes(&disc.meta)?)
        );
        println!(
            "  data_size:     {}",
            human_bytes(device_size_bytes(&disc.data)?)
        );

        match read_meta(&disc.meta) {
            Ok((sb, contract_bytes, contract)) => {
                let (trust, mac_status) = if args.key.is_some() || args.key_file.is_some() {
                    let root = ensure_key_material(args.key.as_deref(), args.key_file.as_deref())?;
                    let (k_mac, _) = root.derive(sb.volume_uuid.as_bytes())?;
                    if verify_mac(&k_mac, &sb, &contract_bytes) {
                        ("MAC verified — trusted", "OK")
                    } else {
                        ("MAC FAILED — untrusted", "FAIL")
                    }
                } else {
                    ("untrusted until MAC verifies (pass --key)", "skipped")
                };

                println!("superblock:      ({trust})");
                println!("  version:       {}.{}", sb.version_major, sb.version_minor);
                println!("  volume_uuid:   {}", sb.volume_uuid);
                println!(
                    "  flags:         {:#x} ({})",
                    sb.flags,
                    sb.flag_names().join("|")
                );
                println!("  generation:    {}", sb.meta_generation);
                println!("  contract_size: {}", sb.contract_size);
                println!("  mac:           {}", hex::encode(sb.mac));
                println!("contract:        ({trust})");
                println!("  format:        {}", contract.format);
                println!("  profile:       {}", contract.profile);
                println!(
                    "  access_mode:   {} (from profile)",
                    contract.profile.access_mode()
                );
                println!(
                    "  key_locator:   {}",
                    contract.key_locator.as_deref().unwrap_or("(none)")
                );
                println!("  integrity:     {}", contract.data.integrity);
                println!("  cipher:        {}", contract.data.cipher);
                if let Some(v) = &contract.verity {
                    println!("  verity.root:   {}", v.root_hash_hex);
                    println!("  verity.alg:    {}", v.algorithm);
                }
                println!(
                    "  has_luks_hdr:  {}",
                    contract.data.luks2_header_b64.is_some()
                );
                println!("mac_verify:      {mac_status}");
            }
            Err(e) => {
                println!("superblock:      unreadable ({e})");
            }
        }
        Ok(())
    })();
    let _ = handle.detach_loop();
    result
}

/// Open a sealed CDF volume. Never formats. Fail-closed on missing/invalid metadata.
pub fn cmd_open(args: OpenArgs) -> Result<()> {
    gpt_layout::require_root()?;
    let root = ensure_key_material(args.key.as_deref(), args.key_file.as_deref())?;

    let handle = DiskHandle::open(&args.disk, true)?;
    let outcome = (|| {
        let disc = gpt_layout::discover(&handle.block_dev)
            .context("not a CDF disk (no CDF GPT partitions); run `create` first")?;
        let inspected = cdf::inspect(disc)
            .context("refusing to open (use `create --force` to re-format if intentional)")?;
        let verified = cdf::verify(inspected, &root)?;
        let activated = cdf::activate(&verified, &args.mapper)?;

        println!("mapped_device:   {}", activated.mapped.display());
        println!("volume_uuid:     {}", verified.volume_uuid);
        println!("profile:         {}", verified.contract.profile);
        println!(
            "access_mode:     {}",
            verified.contract.profile.access_mode()
        );
        println!("block_device:     {}", handle.block_dev.display());
        println!();
        println!("# Stateless teardown (does not modify the disk image):");
        for t in activated.teardown.iter().rev() {
            match t {
                Teardown::Crypt(name) => println!("cryptsetup close {name}"),
                Teardown::Verity(name) => println!("veritysetup close {name}"),
            }
        }
        if handle.owns_loop {
            println!("losetup -d {}", handle.block_dev.display());
        }
        Ok(())
    })();

    if outcome.is_err() {
        let _ = handle.detach_loop();
    }
    outcome
}

fn human_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} {}", U[i])
    } else {
        format!("{v:.2} {}", U[i])
    }
}
