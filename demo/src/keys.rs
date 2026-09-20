//! Key hierarchy from design §6.1:
//!
//! ```text
//! K_root  <-- caller-supplied key bytes
//! K_mac   = HKDF-SHA-256(K_root, salt=volume_uuid, info="cdf-v1/meta-mac")
//! K_luks  = HKDF-SHA-256(K_root, salt=volume_uuid, info="cdf-v1/luks")
//! ```

use anyhow::{bail, Context, Result};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

const INFO_MAC: &[u8] = b"cdf-v1/meta-mac";
const INFO_LUKS: &[u8] = b"cdf-v1/luks";

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct RootKey(pub [u8; 32]);

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MacKey(pub [u8; 32]);

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct LuksKey(pub [u8; 32]);

impl RootKey {
    /// Accept hex (64 nibbles) or arbitrary UTF-8 material hashed with SHA-256.
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            let bytes = hex::decode(trimmed).context("decode --key hex")?;
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            return Ok(Self(out));
        }
        let digest = Sha256::digest(trimmed.as_bytes());
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Ok(Self(out))
    }

    pub fn from_file(path: &str) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("read key file {path}"))?;
        if data.len() == 32 {
            let mut out = [0u8; 32];
            out.copy_from_slice(&data);
            return Ok(Self(out));
        }
        if data.len() == 64 && data.iter().all(|b| b.is_ascii_hexdigit()) {
            return Self::parse(std::str::from_utf8(&data)?.trim());
        }
        // Treat arbitrary file contents like a passphrase: hash to 32 bytes.
        let digest = Sha256::digest(&data);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Ok(Self(out))
    }

    pub fn derive(&self, volume_uuid: &[u8; 16]) -> Result<(MacKey, LuksKey)> {
        Ok((
            MacKey(hkdf_expand(&self.0, volume_uuid, INFO_MAC)?),
            LuksKey(hkdf_expand(&self.0, volume_uuid, INFO_LUKS)?),
        ))
    }
}

fn hkdf_expand(ikm: &[u8], salt: &[u8], info: &[u8]) -> Result<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm)
        .map_err(|_| anyhow::anyhow!("HKDF expand failed"))?;
    Ok(okm)
}

pub fn ensure_key_material(key: Option<&str>, key_file: Option<&str>) -> Result<RootKey> {
    match (key, key_file) {
        (Some(k), None) => RootKey::parse(k),
        (None, Some(p)) => RootKey::from_file(p),
        (Some(_), Some(_)) => bail!("pass only one of --key / --key-file"),
        (None, None) => bail!("missing --key or --key-file"),
    }
}
