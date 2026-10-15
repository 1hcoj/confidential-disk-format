//! Create / open state machine shared by the host tool and the guest opener.
//!
//! Open is split into `inspect` (untrusted read), `verify` (MAC check with
//! `K_root`) and `activate` (device-mapper), so a caller can pick the key
//! locator from the untrusted Contract and apply its own policy to the
//! authenticated identity before anything is mapped.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::device::{device_size_bytes, range_is_zero, run_checked};
use crate::format::{
    decode_luks_header, read_meta, seal_meta, verify_mac, Contract, ContractVerity, Profile,
    Superblock, FLAG_AUTHENTICATED, META_MIB, SUPERBLOCK_SIZE, VERITY_MIB,
};
use crate::gpt_layout::{self, Discovered};
use crate::keys::{LuksKey, RootKey};
use crate::luks;

/// What is on a raw disk, judged without any key and without reading through
/// dm-integrity (where unwritten sectors fail to read).
#[derive(Debug)]
pub enum DiskState {
    /// GPT area, P_meta and the tail are all zero, or CDF partitions exist but
    /// the Superblock was never written (interrupted create). Safe to create.
    Blank,
    /// CDF partitions with a non-zero Superblock. Must be opened, never created over.
    Sealed(Discovered),
    /// Non-zero content that is not a CDF GPT layout.
    Foreign(String),
}

/// GPT header plus P_meta (sgdisk aligns partition 1 at 1 MiB).
const HEAD_BYTES: u64 = (1 + META_MIB) << 20;
/// Backup GPT lives in the last 33 sectors.
const TAIL_BYTES: u64 = 1 << 20;

pub fn probe(block_dev: &Path) -> Result<DiskState> {
    let size = device_size_bytes(block_dev)?;
    if range_is_zero(block_dev, 0, HEAD_BYTES)?
        && range_is_zero(block_dev, size.saturating_sub(TAIL_BYTES), TAIL_BYTES)?
    {
        return Ok(DiskState::Blank);
    }
    match gpt_layout::discover(block_dev) {
        Ok(disc) => {
            if range_is_zero(&disc.meta, 0, SUPERBLOCK_SIZE as u64)? {
                Ok(DiskState::Blank)
            } else {
                Ok(DiskState::Sealed(disc))
            }
        }
        Err(e) => Ok(DiskState::Foreign(format!("{e:#}"))),
    }
}

pub struct CreateRequest<'a> {
    pub block_dev: &'a Path,
    pub profile: Profile,
    pub root: &'a RootKey,
    /// Optional non-secret Contract.key_locator hint.
    pub key_locator: Option<String>,
    /// Finalized payload for verity / verity+crypt.
    pub from_image: Option<&'a Path>,
    /// Runs on the activated data device after LUKS format and before the
    /// metadata is sealed (e.g. mkfs). Only for plain / crypt+integrity.
    pub populate: Option<&'a dyn Fn(&Path) -> Result<()>>,
    /// Allow creating over a disk that is not blank.
    pub force: bool,
}

pub struct Created {
    pub volume_uuid: Uuid,
    pub discovered: Discovered,
}

/// Lay out GPT, format the data partition, run `populate`, then seal P_meta.
/// The Superblock is the last thing written: a crash before it leaves a disk
/// that `probe` reports as `Blank`.
pub fn create(req: CreateRequest<'_>) -> Result<Created> {
    let profile = req.profile;
    match (profile.needs_verity(), req.from_image.is_some()) {
        (true, false) => bail!(
            "profile {profile} requires a finalized image (content must exist before verity seal)"
        ),
        (false, true) => bail!("a source image is only valid for verity / verity+crypt"),
        _ => {}
    }
    if req.populate.is_some() && profile.needs_verity() {
        bail!("populate is not supported for profile {profile}");
    }

    if !req.force {
        match probe(req.block_dev)? {
            DiskState::Blank => {}
            DiskState::Sealed(_) => {
                bail!("disk already holds a CDF volume; refusing to create over it")
            }
            DiskState::Foreign(why) => {
                bail!("disk is not blank and not CDF ({why}); refusing to create over it")
            }
        }
    }

    let size = device_size_bytes(req.block_dev)?;
    let min = (META_MIB
        + if profile.needs_verity() {
            VERITY_MIB
        } else {
            0
        }
        + 64)
        << 20;
    if size < min {
        bail!("disk too small ({size} B); need at least {min} B for profile {profile}");
    }

    gpt_layout::create_partitions(req.block_dev, profile)?;
    let disc = gpt_layout::discover(req.block_dev)?;

    let volume_uuid = Uuid::new_v4();
    let (k_mac, k_luks) = req.root.derive(volume_uuid.as_bytes())?;

    zero_dev(
        &disc.meta,
        (META_MIB << 20).min(device_size_bytes(&disc.meta)?),
    )?;

    let luks_header = match profile {
        Profile::CryptIntegrity => {
            let header = luks::format_detached(&disc.data, &k_luks, &volume_uuid, profile)?;
            if let Some(populate) = req.populate {
                with_mapped(&disc.data, &header, &k_luks, &volume_uuid, populate)?;
            }
            Some(header)
        }
        Profile::VerityCrypt => {
            let from = req
                .from_image
                .context("verity+crypt requires a source image")?;
            let header = luks::format_detached(&disc.data, &k_luks, &volume_uuid, profile)?;
            with_mapped(&disc.data, &header, &k_luks, &volume_uuid, &|mapped| {
                copy_image_to_dev(from, mapped)
            })?;
            Some(header)
        }
        Profile::Verity => {
            let from = req.from_image.context("verity requires a source image")?;
            copy_image_to_dev(from, &disc.data)?;
            None
        }
        Profile::Plain => {
            if let Some(populate) = req.populate {
                populate(&disc.data)?;
            }
            None
        }
    };

    let verity = if profile.needs_verity() {
        let verity_dev = disc
            .verity
            .as_ref()
            .context("profile needs verity but P_verity missing")?;
        Some(verity_format(&disc.data, verity_dev)?)
    } else {
        None
    };

    seal_meta(
        &disc.meta,
        volume_uuid,
        profile,
        &k_mac,
        luks_header.as_deref(),
        verity,
        req.key_locator,
    )?;
    Ok(Created {
        volume_uuid,
        discovered: disc,
    })
}

/// Superblock and Contract as read from disk. Nothing here is trusted until
/// `verify` succeeds; only `volume_uuid` and `contract.key_locator` may be
/// used beforehand (to fetch `K_root`).
pub struct Inspected {
    pub discovered: Discovered,
    pub superblock: Superblock,
    pub contract_bytes: Vec<u8>,
    pub contract: Contract,
}

pub fn inspect(discovered: Discovered) -> Result<Inspected> {
    let (superblock, contract_bytes, contract) =
        read_meta(&discovered.meta).context("CDF Superblock/Contract unreadable")?;
    Ok(Inspected {
        discovered,
        superblock,
        contract_bytes,
        contract,
    })
}

/// An Inspected volume whose metadata authenticated under `K_root`.
pub struct Verified {
    pub discovered: Discovered,
    pub volume_uuid: Uuid,
    pub meta_generation: u64,
    pub contract: Contract,
    k_luks: LuksKey,
}

pub fn verify(inspected: Inspected, root: &RootKey) -> Result<Verified> {
    let Inspected {
        discovered,
        superblock: sb,
        contract_bytes,
        contract,
    } = inspected;
    let (k_mac, k_luks) = root.derive(sb.volume_uuid.as_bytes())?;
    if contract.profile.is_authenticated() {
        if sb.flags & FLAG_AUTHENTICATED == 0 {
            bail!("authenticated profile required but flags.AUTHENTICATED=0");
        }
        if !verify_mac(&k_mac, &sb, &contract_bytes) {
            bail!("MAC verification failed; refusing to open");
        }
    }
    if contract.volume_uuid != sb.volume_uuid || contract.meta_generation != sb.meta_generation {
        bail!("Contract identity fields disagree with Superblock");
    }
    if contract.profile.needs_luks()
        && !contract.data.cipher.is_empty()
        && contract.data.cipher != "aes-xts-plain64"
    {
        bail!("cipher {} not allow-listed", contract.data.cipher);
    }
    Ok(Verified {
        discovered,
        volume_uuid: sb.volume_uuid,
        meta_generation: sb.meta_generation,
        contract,
        k_luks,
    })
}

#[derive(Debug, Clone)]
pub enum Teardown {
    Crypt(String),
    Verity(String),
}

pub struct Activated {
    pub mapped: PathBuf,
    /// Device-mapper targets to close, innermost last.
    pub teardown: Vec<Teardown>,
}

impl Activated {
    pub fn close(&self) -> Result<()> {
        close_all(&self.teardown)
    }
}

fn close_all(teardown: &[Teardown]) -> Result<()> {
    let mut first_err = None;
    for t in teardown.iter().rev() {
        let res = match t {
            Teardown::Crypt(name) => luks::close(name),
            Teardown::Verity(name) => {
                run_checked(Command::new("veritysetup").args(["close", name]))
            }
        };
        if let Err(e) = res {
            first_err.get_or_insert(e);
        }
    }
    first_err.map_or(Ok(()), Err)
}

/// Map the data partition according to the authenticated profile. Never formats.
pub fn activate(v: &Verified, mapper: &str) -> Result<Activated> {
    let disc = &v.discovered;
    let mut teardown = Vec::new();
    let res = (|| -> Result<PathBuf> {
        match v.contract.profile {
            Profile::Plain => Ok(disc.data.clone()),
            Profile::CryptIntegrity => {
                let header = decode_luks_header(&v.contract)?;
                luks::open_detached(&disc.data, &header, &v.k_luks, mapper)?;
                teardown.push(Teardown::Crypt(mapper.to_string()));
                Ok(PathBuf::from(format!("/dev/mapper/{mapper}")))
            }
            Profile::Verity => {
                let root_hash = verity_root(&v.contract)?;
                let verity_dev = disc.verity.as_ref().context("missing verity partition")?;
                run_checked(
                    Command::new("veritysetup")
                        .arg("open")
                        .arg(&disc.data)
                        .arg(mapper)
                        .arg(verity_dev)
                        .arg(root_hash),
                )?;
                teardown.push(Teardown::Verity(mapper.to_string()));
                Ok(PathBuf::from(format!("/dev/mapper/{mapper}")))
            }
            Profile::VerityCrypt => {
                let root_hash = verity_root(&v.contract)?;
                let verity_dev = disc.verity.as_ref().context("missing verity partition")?;
                let verity_mapper = format!("{mapper}-verity");
                run_checked(
                    Command::new("veritysetup")
                        .arg("open")
                        .arg(&disc.data)
                        .arg(&verity_mapper)
                        .arg(verity_dev)
                        .arg(root_hash),
                )?;
                teardown.push(Teardown::Verity(verity_mapper.clone()));
                let header = decode_luks_header(&v.contract)?;
                let verity_path = PathBuf::from(format!("/dev/mapper/{verity_mapper}"));
                luks::open_detached(&verity_path, &header, &v.k_luks, mapper)?;
                teardown.push(Teardown::Crypt(mapper.to_string()));
                Ok(PathBuf::from(format!("/dev/mapper/{mapper}")))
            }
        }
    })();
    match res {
        Ok(mapped) => Ok(Activated { mapped, teardown }),
        Err(e) => {
            let _ = close_all(&teardown);
            Err(e)
        }
    }
}

fn verity_root(contract: &Contract) -> Result<&str> {
    Ok(contract
        .verity
        .as_ref()
        .context("missing verity in Contract")?
        .root_hash_hex
        .as_str())
}

fn with_mapped(
    data_dev: &Path,
    header: &[u8],
    k_luks: &LuksKey,
    volume_uuid: &Uuid,
    f: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    let mapper = format!("cdf-create-{}", volume_uuid.simple());
    luks::open_detached(data_dev, header, k_luks, &mapper)?;
    let res = f(Path::new(&format!("/dev/mapper/{mapper}")));
    let closed = luks::close(&mapper);
    res?;
    closed
}

fn copy_image_to_dev(from: &Path, dest: &Path) -> Result<()> {
    let src_len = device_size_bytes(from)?;
    let dst_len = device_size_bytes(dest)?;
    if src_len > dst_len {
        bail!(
            "source image {} ({src_len} B) does not fit in {} ({dst_len} B)",
            from.display(),
            dest.display()
        );
    }
    run_checked(Command::new("dd").args([
        &format!("if={}", from.display()),
        &format!("of={}", dest.display()),
        "bs=4M",
        "conv=fsync",
        "status=none",
    ]))
}

fn zero_dev(path: &Path, bytes: u64) -> Result<()> {
    use std::io::Write;
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

fn verity_format(data_dev: &Path, hash_dev: &Path) -> Result<ContractVerity> {
    let out = Command::new("veritysetup")
        .args([
            "format",
            "--data-block-size=4096",
            "--hash-block-size=4096",
            "--hash=sha256",
        ])
        .arg(data_dev)
        .arg(hash_dev)
        .output()
        .context("run veritysetup format")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    fn image(size: u64) -> tempfile::NamedTempFile {
        let f = tempfile::NamedTempFile::new().unwrap();
        f.as_file().set_len(size).unwrap();
        f
    }

    #[test]
    fn sparse_file_is_blank() {
        let f = image(256 << 20);
        assert!(matches!(probe(f.path()).unwrap(), DiskState::Blank));
    }

    #[test]
    fn nonzero_head_is_not_blank() {
        let mut f = image(256 << 20);
        f.seek(SeekFrom::Start(4096)).unwrap();
        f.write_all(b"x").unwrap();
        f.flush().unwrap();
        assert!(!matches!(probe(f.path()).unwrap(), DiskState::Blank));
    }

    #[test]
    fn nonzero_tail_is_not_blank() {
        let mut f = image(256 << 20);
        f.seek(SeekFrom::End(-10)).unwrap();
        f.write_all(b"x").unwrap();
        f.flush().unwrap();
        assert!(!matches!(probe(f.path()).unwrap(), DiskState::Blank));
    }
}
