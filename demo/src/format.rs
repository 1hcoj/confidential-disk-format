//! CDF v1 superblock + Contract (design §4.2).

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::io::{Read, Seek, SeekFrom, Write};
use uuid::Uuid;

use crate::keys::MacKey;

pub const MAGIC: &[u8; 8] = b"CDF1\0\0\0\0";
pub const SUPERBLOCK_SIZE: usize = 4096;
pub const CONTRACT_OFFSET: u32 = 4096;

pub const FLAG_AUTHENTICATED: u32 = 1 << 0;
pub const FLAG_HAS_VERITY: u32 = 1 << 1;
pub const FLAG_INITIALIZED: u32 = 1 << 2;

pub const TYPE_META: &str = "4ca88ee1-60f4-4b6b-899e-dae026ced8e8";
pub const TYPE_VERITY: &str = "9620d886-ff76-4ccd-9b09-3a938a18dc0d";
pub const TYPE_DATA: &str = "e4ff66b9-eebf-4675-b8a0-836ca79919ce";

pub const LABEL_META: &str = "cdf-meta";
pub const LABEL_VERITY: &str = "cdf-verity";
pub const LABEL_DATA: &str = "cdf-data";

pub const META_MIB: u64 = 32;
/// Reserved for verity hash tree when the profile needs it.
pub const VERITY_MIB: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    Plain,
    #[serde(rename = "crypt+integrity")]
    CryptIntegrity,
    Verity,
    #[serde(rename = "verity+crypt")]
    VerityCrypt,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::CryptIntegrity => "crypt+integrity",
            Self::Verity => "verity",
            Self::VerityCrypt => "verity+crypt",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "plain" => Self::Plain,
            "crypt+integrity" | "crypt-integrity" | "rw" => Self::CryptIntegrity,
            "verity" | "ro" => Self::Verity,
            "verity+crypt" | "verity-crypt" => Self::VerityCrypt,
            other => bail!("unknown profile {other:?}; expected plain|crypt+integrity|verity|verity+crypt"),
        })
    }

    pub fn needs_luks(self) -> bool {
        matches!(self, Self::CryptIntegrity | Self::VerityCrypt)
    }

    pub fn needs_verity(self) -> bool {
        matches!(self, Self::Verity | Self::VerityCrypt)
    }

    pub fn is_authenticated(self) -> bool {
        !matches!(self, Self::Plain)
    }

    /// Deployment-facing access mode hint (rw vs ro).
    pub fn access_mode(self) -> &'static str {
        match self {
            Self::Plain | Self::CryptIntegrity => "rw",
            Self::Verity | Self::VerityCrypt => "ro",
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub format: String,
    pub volume_uuid: Uuid,
    pub meta_generation: u64,
    pub profile: Profile,
    pub data: ContractData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verity: Option<ContractVerity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub luks2_header_b64: Option<String>,
    pub luks2_subsystem: String,
    pub integrity: String,
    pub cipher: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractVerity {
    pub root_hash_hex: String,
    pub algorithm: String,
    pub data_block_size: u32,
    pub hash_block_size: u32,
    pub salt_hex: String,
}

#[derive(Debug, Clone)]
pub struct Superblock {
    pub version_major: u16,
    pub version_minor: u16,
    pub volume_uuid: Uuid,
    pub flags: u32,
    pub contract_offset: u32,
    pub contract_size: u32,
    pub meta_generation: u64,
    pub mac: [u8; 32],
}

impl Superblock {
    pub fn encode(&self) -> [u8; SUPERBLOCK_SIZE] {
        let mut buf = [0u8; SUPERBLOCK_SIZE];
        buf[0..8].copy_from_slice(MAGIC);
        buf[8..10].copy_from_slice(&self.version_major.to_le_bytes());
        buf[10..12].copy_from_slice(&self.version_minor.to_le_bytes());
        buf[12..28].copy_from_slice(self.volume_uuid.as_bytes());
        buf[28..32].copy_from_slice(&self.flags.to_le_bytes());
        buf[32..36].copy_from_slice(&self.contract_offset.to_le_bytes());
        buf[36..40].copy_from_slice(&self.contract_size.to_le_bytes());
        buf[40..48].copy_from_slice(&self.meta_generation.to_le_bytes());
        buf[48..80].copy_from_slice(&self.mac);
        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < SUPERBLOCK_SIZE {
            bail!("superblock too short");
        }
        if &buf[0..8] != MAGIC {
            bail!("not a CDF v1 superblock (bad magic)");
        }
        let version_major = u16::from_le_bytes(buf[8..10].try_into()?);
        if version_major != 1 {
            bail!("unsupported CDF major version {version_major}");
        }
        let version_minor = u16::from_le_bytes(buf[10..12].try_into()?);
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&buf[12..28]);
        let volume_uuid = Uuid::from_bytes(uuid_bytes);
        let flags = u32::from_le_bytes(buf[28..32].try_into()?);
        let contract_offset = u32::from_le_bytes(buf[32..36].try_into()?);
        let contract_size = u32::from_le_bytes(buf[36..40].try_into()?);
        let meta_generation = u64::from_le_bytes(buf[40..48].try_into()?);
        let mut mac = [0u8; 32];
        mac.copy_from_slice(&buf[48..80]);
        Ok(Self {
            version_major,
            version_minor,
            volume_uuid,
            flags,
            contract_offset,
            contract_size,
            meta_generation,
            mac,
        })
    }

    pub fn flag_names(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.flags & FLAG_AUTHENTICATED != 0 {
            v.push("AUTHENTICATED");
        }
        if self.flags & FLAG_HAS_VERITY != 0 {
            v.push("HAS_VERITY");
        }
        if self.flags & FLAG_INITIALIZED != 0 {
            v.push("INITIALIZED");
        }
        v
    }
}

pub fn compute_mac(k_mac: &MacKey, sb: &Superblock, contract: &[u8]) -> [u8; 32] {
    let mut sb_bytes = sb.encode();
    sb_bytes[48..80].fill(0);
    let mut mac = Hmac::<Sha256>::new_from_slice(&k_mac.0).expect("HMAC key length");
    mac.update(&sb_bytes);
    mac.update(contract);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

pub fn verify_mac(k_mac: &MacKey, sb: &Superblock, contract: &[u8]) -> bool {
    let expected = compute_mac(k_mac, sb, contract);
    // Constant-time compare
    expected
        .iter()
        .zip(sb.mac.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

pub fn serialize_contract(c: &Contract) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(c)?)
}

pub fn parse_contract(bytes: &[u8]) -> Result<Contract> {
    serde_json::from_slice(bytes).context("parse CDF Contract JSON")
}

pub fn read_meta(meta_dev: &std::path::Path) -> Result<(Superblock, Vec<u8>, Contract)> {
    let mut f = std::fs::File::open(meta_dev).with_context(|| format!("open {}", meta_dev.display()))?;
    let mut sb_buf = [0u8; SUPERBLOCK_SIZE];
    f.read_exact(&mut sb_buf)?;
    if sb_buf.iter().all(|b| *b == 0) {
        bail!("meta partition is empty (all zeros)");
    }
    let sb = Superblock::decode(&sb_buf)?;
    if sb.contract_offset as usize != CONTRACT_OFFSET as usize {
        bail!(
            "unexpected contract_offset {} (demo expects {CONTRACT_OFFSET})",
            sb.contract_offset
        );
    }
    if sb.contract_size == 0 || sb.contract_size > 28 * 1024 * 1024 {
        bail!("implausible contract_size {}", sb.contract_size);
    }
    f.seek(SeekFrom::Start(sb.contract_offset as u64))?;
    let mut contract_bytes = vec![0u8; sb.contract_size as usize];
    f.read_exact(&mut contract_bytes)?;
    let contract = parse_contract(&contract_bytes)?;
    Ok((sb, contract_bytes, contract))
}

pub fn write_meta(
    meta_dev: &std::path::Path,
    sb: &Superblock,
    contract_bytes: &[u8],
) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(meta_dev)
        .with_context(|| format!("open {} for write", meta_dev.display()))?;
    // Contract first, Superblock last (design §8.2).
    f.seek(SeekFrom::Start(sb.contract_offset as u64))?;
    f.write_all(contract_bytes)?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&sb.encode())?;
    f.sync_all()?;
    Ok(())
}

pub fn seal_meta(
    meta_dev: &std::path::Path,
    volume_uuid: Uuid,
    profile: Profile,
    k_mac: &MacKey,
    luks_header: Option<&[u8]>,
    verity: Option<ContractVerity>,
) -> Result<(Superblock, Contract)> {
    let mut flags = FLAG_INITIALIZED;
    if profile.is_authenticated() {
        flags |= FLAG_AUTHENTICATED;
    }
    if profile.needs_verity() {
        flags |= FLAG_HAS_VERITY;
    }

    let integrity = if profile.needs_luks() {
        "hmac-sha256"
    } else {
        "none"
    };

    let contract = Contract {
        format: "cdf/v1".into(),
        volume_uuid,
        meta_generation: 1,
        profile,
        data: ContractData {
            luks2_header_b64: luks_header.map(|h| B64.encode(h)),
            luks2_subsystem: "cdf-v1".into(),
            integrity: integrity.into(),
            cipher: "aes-xts-plain64".into(),
        },
        verity,
    };
    let contract_bytes = serialize_contract(&contract)?;

    let mut sb = Superblock {
        version_major: 1,
        version_minor: 0,
        volume_uuid,
        flags,
        contract_offset: CONTRACT_OFFSET,
        contract_size: contract_bytes.len() as u32,
        meta_generation: 1,
        mac: [0u8; 32],
    };
    sb.mac = compute_mac(k_mac, &sb, &contract_bytes);
    write_meta(meta_dev, &sb, &contract_bytes)?;
    Ok((sb, contract))
}

pub fn decode_luks_header(contract: &Contract) -> Result<Vec<u8>> {
    let b64 = contract
        .data
        .luks2_header_b64
        .as_ref()
        .context("Contract missing luks2_header_b64")?;
    B64.decode(b64).context("decode luks2_header_b64")
}
