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
    /// 32 raw bytes, 64 hex characters, or arbitrary material hashed with SHA-256.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() == 32 {
            let mut out = [0u8; 32];
            out.copy_from_slice(data);
            return Ok(Self(out));
        }
        let trimmed = std::str::from_utf8(data).unwrap_or("").trim();
        if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            let bytes = hex::decode(trimmed).context("decode hex K_root")?;
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            return Ok(Self(out));
        }
        let digest = Sha256::digest(data);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Ok(Self(out))
    }

    /// Accept hex (64 nibbles) or arbitrary UTF-8 material hashed with SHA-256.
    pub fn parse(input: &str) -> Result<Self> {
        Self::from_bytes(input.trim().as_bytes())
    }

    pub fn from_file(path: &str) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("read key file {path}"))?;
        Self::from_bytes(&data)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_and_hex_key_material_agree() {
        let raw = [7u8; 32];
        let hex_str = hex::encode(raw);
        assert_eq!(RootKey::from_bytes(&raw).unwrap().0, raw);
        assert_eq!(RootKey::from_bytes(hex_str.as_bytes()).unwrap().0, raw);
        assert_eq!(RootKey::parse(&hex_str).unwrap().0, raw);
    }

    #[test]
    fn derived_keys_depend_on_volume_uuid() {
        let root = RootKey([1u8; 32]);
        let (mac_a, luks_a) = root.derive(&[0u8; 16]).unwrap();
        let (mac_b, _) = root.derive(&[1u8; 16]).unwrap();
        assert_ne!(mac_a.0, luks_a.0);
        assert_ne!(mac_a.0, mac_b.0);
    }
}
