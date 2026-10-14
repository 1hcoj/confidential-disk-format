//! High-level create / info / open operations.
//!
//! State machine:
//!   S0 (raw / non-CDF) --create[--force]--> S1 (sealed CDF GPT)
//!   S1 --open--> S2 (mapped; teardown commands printed; disk unchanged)

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::device::{
    device_size_bytes, ensure_image_file, is_block_device, parse_size, run_checked, DiskHandle,
};
use crate::format::{
    decode_luks_header, read_meta, seal_meta, verify_mac, ContractVerity, Profile, MAGIC, META_MIB,
    VERITY_MIB,
};
use crate::gpt_layout::{self, Discovered};
use crate::keys::{ensure_key_material, LuksKey, RootKey};

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

    // Profile-gated content source (ro verity* must freeze content before seal).
    match (args.profile.needs_verity(), &args.from_image) {
        (true, None) => {
            bail!(
                "profile {} requires --from-image <path> (content must be finalized before verity seal)",
                args.profile
            );
        }
        (false, Some(_)) => {
            bail!(
                "--from-image is only valid for verity / verity+crypt (got profile {})",
                args.profile
            );
        }
        _ => {}
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
        if cdf_already_present(&handle.block_dev)? {
            if !args.force {
                bail!(
                    "disk already looks like a CDF volume; refusing to overwrite \
                     (pass --force to re-format)"
                );
            }
            eprintln!(
                "warning: --force: destroying existing CDF layout on {}",
                args.disk.display()
            );
        }

        let layout = gpt_layout::create_partitions(&handle.block_dev, args.profile)?;
        let disc = gpt_layout::discover(&handle.block_dev)?;
        format_and_seal(
            &disc,
            args.profile,
            &root,
            args.from_image.as_deref(),
            args.key_locator.clone(),
        )?;
        println!("created CDF image");
        println!("  disk:          {}", args.disk.display());
        println!("  block:         {}", handle.block_dev.display());
        println!("  profile:       {}", args.profile);
        println!("  access_mode:   {}", args.profile.access_mode());
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
            layout.meta_index, layout.data_index
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

        let disc = match gpt_layout::discover(&handle.block_dev) {
            Ok(d) => d,
            Err(e) => {
                println!("cdf:             (no CDF GPT partitions: {e})");
                return Ok(());
            }
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
                // Trust label depends on whether we verified the MAC.
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
    let mut cleanup = CleanupPrinter::new();
    if handle.owns_loop {
        cleanup.note_loop(&handle.block_dev);
    }

    let outcome = (|| {
        let disc = gpt_layout::discover(&handle.block_dev)
            .context("not a CDF disk (no CDF GPT partitions); run `create` first")?;
        let (sb, contract_bytes, contract) = read_meta(&disc.meta).context(
            "CDF partitions found but Superblock/Contract unreadable; \
             refusing to open (use `create --force` to re-format if intentional)",
        )?;

        let (k_mac, k_luks) = root.derive(sb.volume_uuid.as_bytes())?;
        if contract.profile.is_authenticated() {
            if sb.flags & crate::format::FLAG_AUTHENTICATED == 0 {
                bail!("authenticated profile required but flags.AUTHENTICATED=0");
            }
            if !verify_mac(&k_mac, &sb, &contract_bytes) {
                bail!("MAC verification failed; refusing to open");
            }
        }
        if contract.volume_uuid != sb.volume_uuid || contract.meta_generation != sb.meta_generation
        {
            bail!("Contract identity fields disagree with Superblock");
        }
        if contract.profile.needs_luks() && contract.data.cipher != "aes-xts-plain64" {
            bail!("cipher {} not allow-listed", contract.data.cipher);
        }

        let mapped = activate(&disc, &contract, &k_luks, &args.mapper, &mut cleanup)?;

        println!("mapped_device:   {mapped}");
        println!("volume_uuid:     {}", sb.volume_uuid);
        println!("profile:         {}", contract.profile);
        println!("access_mode:     {}", contract.profile.access_mode());
        println!("block_device:     {}", handle.block_dev.display());
        println!();
        cleanup.print();
        Ok(())
    })();

    if outcome.is_err() {
        let _ = Command::new("cryptsetup")
            .args(["close", &args.mapper])
            .status();
        let verity_mapper = format!("{}-verity", args.mapper);
        let _ = Command::new("veritysetup")
            .args(["close", &args.mapper])
            .status();
        let _ = Command::new("veritysetup")
            .args(["close", &verity_mapper])
            .status();
        let _ = handle.detach_loop();
    }
    outcome
}

/// True if GPT already has CDF type UUIDs and P_meta starts with CDF magic.
fn cdf_already_present(block_dev: &Path) -> Result<bool> {
    let disc = match gpt_layout::discover(block_dev) {
        Ok(d) => d,
        Err(_) => return Ok(false),
    };
    let mut f =
        std::fs::File::open(&disc.meta).with_context(|| format!("open {}", disc.meta.display()))?;
    let mut magic = [0u8; 8];
    match f.read_exact(&mut magic) {
        Ok(()) => Ok(magic == *MAGIC),
        Err(_) => Ok(false),
    }
}

struct CleanupPrinter {
    closes: Vec<String>,
    loop_dev: Option<String>,
}

impl CleanupPrinter {
    fn new() -> Self {
        Self {
            closes: Vec::new(),
            loop_dev: None,
        }
    }

    fn note_loop(&mut self, loop_dev: &Path) {
        self.loop_dev = Some(format!("losetup -d {}", loop_dev.display()));
    }

    fn note_cryptsetup_close(&mut self, name: &str) {
        self.closes.push(format!("cryptsetup close {name}"));
    }

    fn note_veritysetup_close(&mut self, name: &str) {
        self.closes.push(format!("veritysetup close {name}"));
    }

    fn print(&self) {
        println!("# Stateless teardown (does not modify the disk image):");
        for line in self.closes.iter().rev() {
            println!("{line}");
        }
        if let Some(l) = &self.loop_dev {
            println!("{l}");
        }
    }
}

/// Feed `K_luks` to cryptsetup via stdin (`--key-file -`); never writes key material to disk.
fn cryptsetup_with_key(mut cmd: Command, key: &[u8]) -> Result<()> {
    use std::process::Stdio;
    let prog = format!("{cmd:?}");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().with_context(|| format!("spawn {prog}"))?;
    {
        let mut stdin = child.stdin.take().context("cryptsetup stdin")?;
        stdin.write_all(key)?;
    }
    let out = child
        .wait_with_output()
        .with_context(|| format!("wait {prog}"))?;
    if !out.status.success() {
        bail!(
            "command failed: {prog}\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Detached-header open: header lives in an ephemeral temp file; key goes via stdin.
/// After open succeeds the mapping no longer needs the header file.
fn cryptsetup_open_detached(
    data_dev: &Path,
    header: &[u8],
    k_luks: &LuksKey,
    mapper: &str,
) -> Result<()> {
    let mut hdr = tempfile::NamedTempFile::new().context("temp LUKS header")?;
    hdr.write_all(header)?;
    hdr.flush()?;
    let hdr_path = hdr.path().to_path_buf();

    let mut cmd = Command::new("cryptsetup");
    cmd.args(["open", "--type", "luks2", "--header"])
        .arg(&hdr_path)
        .args(["--key-file", "-"])
        .arg(data_dev)
        .arg(mapper);
    cryptsetup_with_key(cmd, &k_luks.0)?;
    drop(hdr);
    Ok(())
}

fn format_and_seal(
    disc: &Discovered,
    profile: Profile,
    root: &RootKey,
    from_image: Option<&Path>,
    key_locator: Option<String>,
) -> Result<()> {
    let volume_uuid = Uuid::new_v4();
    let (k_mac, k_luks) = root.derive(volume_uuid.as_bytes())?;

    zero_dev(
        &disc.meta,
        (META_MIB << 20).min(device_size_bytes(&disc.meta)?),
    )?;

    let luks_header = match profile {
        Profile::CryptIntegrity => Some(luks_format_detached(
            &disc.data,
            &k_luks,
            &volume_uuid,
            profile,
        )?),
        Profile::VerityCrypt => {
            let from = from_image.context("verity+crypt requires --from-image")?;
            let header = luks_format_detached(&disc.data, &k_luks, &volume_uuid, profile)?;
            // Populate ciphertext via a temporary open, then close before verity format.
            populate_luks_payload(&disc.data, &header, &k_luks, from)?;
            Some(header)
        }
        Profile::Verity => {
            let from = from_image.context("verity requires --from-image")?;
            copy_image_to_dev(from, &disc.data)?;
            None
        }
        Profile::Plain => None,
    };

    let verity = if profile.needs_verity() {
        let verity_dev = disc
            .verity
            .as_ref()
            .context("profile needs verity but P_verity missing")?;
        // For VerityCrypt, hash the LUKS ciphertext on P_data.
        Some(verity_format(&disc.data, verity_dev)?)
    } else {
        None
    };

    let (_sb, _contract) = seal_meta(
        &disc.meta,
        volume_uuid,
        profile,
        &k_mac,
        luks_header.as_deref(),
        verity,
        key_locator,
    )?;
    eprintln!("sealed volume_uuid={volume_uuid} profile={profile} generation=1");
    Ok(())
}

fn populate_luks_payload(
    data_dev: &Path,
    header: &[u8],
    k_luks: &LuksKey,
    from_image: &Path,
) -> Result<()> {
    let mapper = format!("cdf-populate-{}", std::process::id());
    let open = (|| {
        cryptsetup_open_detached(data_dev, header, k_luks, &mapper)?;
        let mapped = PathBuf::from(format!("/dev/mapper/{mapper}"));
        copy_image_to_dev(from_image, &mapped)?;
        Ok(())
    })();
    let _ = Command::new("cryptsetup").args(["close", &mapper]).status();
    open
}

fn copy_image_to_dev(from: &Path, dest: &Path) -> Result<()> {
    let src_len = device_size_bytes(from)
        .or_else(|_| Ok::<u64, anyhow::Error>(std::fs::metadata(from)?.len()))?;
    let dst_len = device_size_bytes(dest)?;
    if src_len > dst_len {
        bail!(
            "--from-image {} ({} B) does not fit in data device {} ({} B)",
            from.display(),
            src_len,
            dest.display(),
            dst_len
        );
    }
    eprintln!(
        "copying {} ({} B) → {}",
        from.display(),
        src_len,
        dest.display()
    );
    // Prefer dd for sparse-friendly block copies.
    run_checked(&mut Command::new("dd").args([
        &format!("if={}", from.display()),
        &format!("of={}", dest.display()),
        "bs=4M",
        "conv=fsync",
        "status=none",
    ]))?;
    Ok(())
}

fn zero_dev(path: &Path, bytes: u64) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("open {} to zero", path.display()))?;
    let chunk = vec![0u8; 1024 * 1024];
    let mut left = bytes;
    while left > 0 {
        let n = (left as usize).min(chunk.len());
        f.write_all(&chunk[..n])?;
        left -= n as u64;
    }
    f.sync_all()?;
    Ok(())
}

fn luks_format_detached(
    data_dev: &Path,
    k_luks: &LuksKey,
    volume_uuid: &Uuid,
    profile: Profile,
) -> Result<Vec<u8>> {
    let dir = tempfile::tempdir().context("tempdir for LUKS header")?;
    let header = dir.path().join("luks2.header");
    {
        let f = std::fs::File::create(&header)?;
        // 4 MiB detached header; base64 ≈ 5.3 MiB inside P_meta (32 MiB).
        f.set_len(4 * 1024 * 1024)?;
    }

    let mut cmd = Command::new("cryptsetup");
    cmd.args([
        "luksFormat",
        "--type",
        "luks2",
        "--batch-mode",
        "--cipher",
        "aes-xts-plain64",
        "--key-size",
        "512",
        "--sector-size",
        "4096",
        "--pbkdf",
        "pbkdf2",
        "--pbkdf-force-iterations",
        "1000",
        "--subsystem",
        "cdf-v1",
        "--label",
        &volume_uuid.to_string(),
        "--header",
    ]);
    cmd.arg(&header);
    if matches!(profile, Profile::CryptIntegrity) {
        cmd.args(["--integrity", "hmac-sha256", "--integrity-no-wipe"]);
    }
    // Key via stdin — not written to a keyfile on disk.
    cmd.args(["--key-file", "-"]);
    cmd.arg(data_dev);
    cryptsetup_with_key(cmd, &k_luks.0)?;

    let bytes = std::fs::read(&header).context("read detached LUKS2 header")?;
    if !bytes.windows(4).any(|w| w == b"LUKS") {
        bail!("detached header missing LUKS magic");
    }
    Ok(bytes)
}

fn verity_format(data_dev: &Path, hash_dev: &Path) -> Result<ContractVerity> {
    let mut cmd = Command::new("veritysetup");
    cmd.args([
        "format",
        "--data-block-size=4096",
        "--hash-block-size=4096",
        "--hash=sha256",
    ])
    .arg(data_dev)
    .arg(hash_dev);
    let out = cmd.output().context("run veritysetup format")?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        bail!("veritysetup format failed:\n{combined}");
    }
    let mut root = None;
    let mut salt = String::new();
    for line in combined.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("Root hash:") {
            root = Some(v.trim().to_string());
        }
        if let Some(v) = line.strip_prefix("Salt:") {
            salt = v.trim().to_string();
        }
    }
    let root_hash_hex = root.context(format!(
        "could not parse veritysetup root hash from:\n{combined}"
    ))?;
    Ok(ContractVerity {
        root_hash_hex,
        algorithm: "sha256".into(),
        data_block_size: 4096,
        hash_block_size: 4096,
        salt_hex: salt,
    })
}

fn activate(
    disc: &Discovered,
    contract: &crate::format::Contract,
    k_luks: &LuksKey,
    mapper: &str,
    cleanup: &mut CleanupPrinter,
) -> Result<String> {
    match contract.profile {
        Profile::Plain => Ok(disc.data.display().to_string()),
        Profile::CryptIntegrity => open_luks(disc, contract, k_luks, mapper, cleanup),
        Profile::Verity => {
            let verity_dev = disc.verity.as_ref().context("missing verity partition")?;
            let v = contract
                .verity
                .as_ref()
                .context("missing verity in Contract")?;
            run_checked(
                &mut Command::new("veritysetup")
                    .args(["open"])
                    .arg(&disc.data)
                    .arg(mapper)
                    .arg(verity_dev)
                    .arg(&v.root_hash_hex),
            )?;
            cleanup.note_veritysetup_close(mapper);
            Ok(format!("/dev/mapper/{mapper}"))
        }
        Profile::VerityCrypt => {
            let verity_dev = disc.verity.as_ref().context("missing verity partition")?;
            let v = contract
                .verity
                .as_ref()
                .context("missing verity in Contract")?;
            let verity_mapper = format!("{mapper}-verity");
            run_checked(
                &mut Command::new("veritysetup")
                    .args(["open"])
                    .arg(&disc.data)
                    .arg(&verity_mapper)
                    .arg(verity_dev)
                    .arg(&v.root_hash_hex),
            )?;
            cleanup.note_veritysetup_close(&verity_mapper);

            let header_bytes = decode_luks_header(contract)?;
            let verity_path = PathBuf::from(format!("/dev/mapper/{verity_mapper}"));
            cryptsetup_open_detached(&verity_path, &header_bytes, k_luks, mapper)?;
            cleanup.note_cryptsetup_close(mapper);
            Ok(format!("/dev/mapper/{mapper}"))
        }
    }
}

fn open_luks(
    disc: &Discovered,
    contract: &crate::format::Contract,
    k_luks: &LuksKey,
    mapper: &str,
    cleanup: &mut CleanupPrinter,
) -> Result<String> {
    let header_bytes = decode_luks_header(contract)?;
    cryptsetup_open_detached(&disc.data, &header_bytes, k_luks, mapper)?;
    cleanup.note_cryptsetup_close(mapper);
    Ok(format!("/dev/mapper/{mapper}"))
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
