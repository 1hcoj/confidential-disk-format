# CDF v1 — Confidential Disk Format

**Status.** Draft specification.  
**Date.** 2026-09-10.  
**Survey.** [investigate/README.md](../investigate/README.md) ([中文](../investigate/README.zh.md)).  
**中文译本.** [README.zh.md](README.zh.md).

This document is the on-disk contract. It is intended to be opened by a CoCo guest (CDH / Trustee) and by a standalone CVM that does not run Kata.

---

## 1. Verdict: no existing standard fits

The repository goals are R1–R5 ([survey §1](../investigate/README.md)): optional crypt / integrity, CoCo **and** CVM without a sidecar, self-describing image, Linux dm-crypt out of the box, and an **authenticated** open contract.

No surveyed format satisfies all of them. In the survey's taxonomy ([§5](../investigate/README.md#5-formats-grouped-by-class)), CDF v1 is **Class M + Class E**: partition-table discovery combined with an authenticated region. No existing project combines those two. Every data-plane object CDF wraps is an existing standard (see §2).

| Candidate | Why it is not the format |
|-----------|--------------------------|
| **LUKS2** | Self-describing crypt container ([spec](https://gitlab.com/cryptsetup/LUKS2-docs)), but the header is not authentic against a host. Fails R5; fails the ToB CVM threat model. |
| **LUKS2 + `kbs` token** ([libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs)) | Puts a KBS locator on the disk. Header still unauthenticated; needs a cryptsetup plugin; no verity or GPT discovery. |
| **systemd-homed** ([HOME_DIRECTORY](https://systemd.io/HOME_DIRECTORY/)) | Best portable `GPT · LUKS2 · token` image. Metadata is a desktop user record, not attestation + KBS. Header still host-malleable. |
| **Clevis / systemd-cryptenroll** | On-disk “how to get the key”, not a CVM volume image. |
| **cryptpilot** | Productized CVM + KBS + optional integrity / verity. Key URI and verity root live in **guest TOML / initrd**, so the disk is not self-describing (R2, R3). |
| **tdx-init** | Correct envelope (MAC + detached open + fail-closed). Implementation-specific layout, not a portable GPT image, no verity partition or type UUIDs. |
| **Constellation / Contrast / dstack** | Detached header + JSON allow-list (or MAC). Cluster-local keys, no portable spec. |
| **Azure / GCP / IBM SE / qcow2+LUKS** | Platform or VM-image contracts, not a raw volume you can attach anywhere. |
| **VeraCrypt, BitLocker, TCG Opal, ZFS, fscrypt, ocicrypt** | Wrong data plane for “`cryptsetup` + CDH `SecureMount`” (R4). |

**Reuse, do not replace:** LUKS2, `dm-integrity`, `dm-verity`, GPT.  
**Invent:** a GPT-discoverable meta partition whose open parameters are **MAC'd** under a root key the host does not have. Nothing in it is secret, so it is authenticated but not encrypted (§7.1).

---

## 2. Provenance (what is borrowed)

| Piece | Source | Role in CDF |
|-------|--------|-------------|
| GPT discovery by type UUID + label | [UEFI GPT](https://uefi.org/specs/UEFI/2.10/05_GUID_Partition_Table_Format.html); pattern from [systemd-homed](https://systemd.io/HOME_DIRECTORY/) | Find `P_meta` / `P_verity` / `P_data` without CoCo |
| LUKS2 container + tokens | [LUKS2 On-Disk Format](https://gitlab.com/cryptsetup/LUKS2-docs) | Confidential RW payload |
| Detached header, `open --header` | [cryptsetup](https://gitlab.com/cryptsetup/cryptsetup); ToB CVM guidance | Close TOCTOU; data partition holds payload only |
| `dm-integrity` (`hmac-sha256`) | [kernel dm-integrity](https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html); cryptsetup `--integrity` | Mutable tamper evidence |
| `dm-verity` + 512 B superblock | [kernel dm-verity](https://docs.kernel.org/admin-guide/device-mapper/verity.html); [cryptsetup wiki](https://gitlab.com/cryptsetup/cryptsetup/-/wikis/DMVerity) | Immutable RO tree |
| MAC over the header, cleartext header | [Trail of Bits (2025-10-30)](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/); [tdx-init v0.2.0](https://github.com/flashbots/tdx-init); [CDH #1648](https://github.com/confidential-containers/guest-components/pull/1648) | Authenticate the open contract. Both precedents MAC a cleartext header rather than encrypting it |
| Fail-closed reopen; no format-on-unlock-failure | [CDH #1648](https://github.com/confidential-containers/guest-components/pull/1648) | Existing volumes must not become from-scratch |
| Journaled integrity for persistent LUKS | [CDH #1663](https://github.com/confidential-containers/guest-components/pull/1663) | The **only** confidential RW profile: encryption is never offered without integrity (§5) |
| Key locator kept **off** the image | [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs) (SMBIOS `initdata.toml` preferred over on-disk fields); [Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) (manifest URI in measured policy) | §6.2: the image carries identity, the deployment carries the locator |
| HKDF-split root key | Common KDF practice ([HKDF / RFC 5869](https://www.rfc-editor.org/rfc/rfc5869)) | `K_mac` must not be derived by parsing an untrusted LUKS header |
| Metadata generation counter | Ordinary versioned-metadata practice | Reject a stale Contract re-pasted under the same `K_mac` (§8.4) |

---

## 3. Threat model and security goals

**Adversary.** The host / storage stack can read and write every byte of the image. It does not obtain `K_root` (released only after attestation, or supplied by the CVM owner on a trusted path).

**Goals.**

| ID | Goal | How CDF addresses it |
|----|------|----------------------|
| G1 | Optional confidentiality / integrity | Profiles in §5; recorded in the MAC-covered Contract |
| G2 | CoCo and CVM | GPT type UUIDs + on-image locator; no Kata-only sidecar required |
| G3 | Self-describing | One cleartext Contract: readable for discovery, authoritative only after its MAC verifies |
| G4 | Out-of-box data plane | `cryptsetup` + device-mapper only |
| G5 | Header replace must not succeed | HMAC under `K_mac` over the whole contract; detached open from the verified header |

**Non-goals.**

- **Freshness of the whole device (rollback / replay).** The host may substitute an older, internally consistent copy of the image: every `dm-integrity` tag verifies and the Contract MAC verifies, because both were produced under the same keys. CDF does **not** detect this, and deliberately specifies no mechanism for it. Doing so requires mutable state outside the disk (a stateful key broker that remembers the newest generation per volume, or a guest-visible monotonic counter), which upstream Trustee does not provide; [Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) excludes the same case for the same reason ("neither the disk nor Trustee records which authenticated state is newest"). Deployments needing it must add an external freshness anchor and treat that as a layer above this format.
- Intra-session replay of individual sectors. `dm-integrity` tags bind the sector number, not a write version, so replaying an old `(sector, tag)` pair under the same key is not detected.
- RW-many fencing; hiding that the disk is a CDF image; plausible deniability; trusting disk firmware (OPAL).

Header replace that makes `open` **fail** is availability, not a confidentiality break of old AES-XTS ciphertext. The case CDF forbids is `open` **succeeding** with a host-chosen data plane (new writes, or format-after-failed-mount, become host-visible). That applies to existing volumes as well as from-scratch disks.

---

## 4. On-disk format

### 4.1 Image

```
Image ::= GPT( P_meta [, P_verity], P_data )

P_meta.type_uuid    = 4ca88ee1-60f4-4b6b-899e-dae026ced8e8   -- "CDF meta"
P_verity.type_uuid  = 9620d886-ff76-4ccd-9b09-3a938a18dc0d   -- "CDF verity"
P_data.type_uuid    = e4ff66b9-eebf-4675-b8a0-836ca79919ce   -- "CDF data"

P_meta.label        = "cdf-meta"
P_verity.label      = "cdf-verity"          -- absent if no verity
P_data.label        = "cdf-data"
```

`P_meta` SHOULD be at least 32 MiB (a full LUKS2 header backup is commonly 16 MiB).  
`P_verity` is omitted unless the profile uses verity.  
`P_data` is the rest of the usable capacity.

```
+--------+-------------+----------------------+---------------------------+
| GPT    | P_meta      | [P_verity]           | P_data                    |
|        | CDF envelope| verity SB + HashTree | payload (see profile)     |
+--------+-------------+----------------------+---------------------------+
```

### 4.2 Meta partition

All multi-byte integers in the superblock are **little-endian**. Unused bytes are zero.

The meta partition is **cleartext and authenticated**. There is exactly one copy of the open contract; it is readable without any key, and it is only *usable* after its MAC verifies. See §7.1 for why authentication without encryption is the right choice here.

```
P_meta ::= Superblock_4096 · Contract · Pad

Superblock ::=
    magic[8]           = "CDF1" || 0x00 0x00 0x00 0x00
    version_major      : u16        -- 1
    version_minor      : u16        -- 0
    volume_uuid        : u8[16]     -- RFC 4122 binary UUID
    flags              : u32        -- see below
    contract_offset    : u32        -- from start of P_meta; typically 4096
    contract_size      : u32        -- exact byte count of Contract
    meta_generation    : u64        -- starts at 1; incremented on each Contract rewrite
    mac                : u8[32]     -- HMAC-SHA-256; zero-filled while computing
    reserved           : zero to 4096
```

`flags` (OR):

| Bit | Name | Meaning |
|-----|------|---------|
| 0 | `AUTHENTICATED` | `mac` is present and MUST verify before any field of Contract is used |
| 1 | `HAS_VERITY` | `P_verity` is present |
| 2 | `INITIALIZED` | Set after the first successful format |

The flag is a **hint about this image, not a security decision**: the host can clear it. Whether a volume is *required* to be authenticated comes from the deployment (measured policy, CoCo manifest, CVM owner config). An opener that expects a confidential volume MUST refuse an image presenting `flags.AUTHENTICATED == 0`, exactly as it would refuse a failed MAC.

**MAC**

```
mac = HMAC-SHA-256( K_mac,
                    Superblock[ mac := 0^32 ]              -- all 4096 bytes
                    || Contract[ 0 .. contract_size ) )
```

Covering the whole superblock authenticates `volume_uuid`, `flags`, `meta_generation`, and the offset/size pair, so the host can neither relocate nor truncate the contract, nor reinterpret which bytes are covered. `contract_size` is authenticated, which removes any concatenation ambiguity between the two inputs. Comparison MUST be constant-time.

`meta_generation` is inside the MAC input, so a contract written for one generation cannot be pasted over a later one. This is scoped to the metadata; it is **not** a freshness guarantee for the device, which is an explicit non-goal (§3).

**Contract** (cleartext JSON, exactly `contract_size` bytes, no trailing NUL required)

```json
{
  "format": "cdf/v1",
  "volume_uuid": "<same as superblock>",
  "meta_generation": 1,
  "profile": "plain" | "crypt+integrity" | "verity" | "verity+crypt",
  "data": {
    "luks2_header_b64": "<detached LUKS2 header, if profile uses LUKS>",
    "luks2_subsystem": "cdf-v1",
    "integrity": "none" | "hmac-sha256",
    "cipher": "aes-xts-plain64"
  },
  "verity": {
    "root_hash_hex": "...",
    "algorithm": "sha256",
    "data_block_size": 4096,
    "hash_block_size": 4096,
    "salt_hex": "..."
  }
}
```

`verity` is required iff `profile` contains `verity`. `luks2_header_b64` is required iff `profile` contains `crypt`. After the MAC verifies, `volume_uuid` and `meta_generation` MUST equal the superblock (they are duplicated so that a parser reading only the JSON cannot be desynchronised from the binary header).

**The Contract MUST NOT contain a secret.** This is a normative constraint, not an observation: it is what makes a MAC sufficient. A future version that needs to carry a secret field must define an encrypted extension rather than putting it here.

**No key locator on the image.** There is deliberately no `kbs_uri` field; see §6.2.

### 4.3 Data and verity partitions

```
-- profile plain
P_data ::= Filesystem

-- profile crypt+integrity
P_data ::= Payload                        -- NO live LUKS header
           cryptsetup open --type luks2 --header H_ram P_data
           H_ram = decode(Contract.data.luks2_header_b64)

-- profile verity | verity+crypt
P_verity ::= VeritySB_512 · PadTo(hash_block) · HashTree
P_data   ::= DataBlocks | LUKS2_Payload
             root hash taken from Contract.verity, never from P_verity alone
```

LUKS2 on `P_data` is created with a **detached** header (`cryptsetup luksFormat --header …`). The on-disk `P_data` start is ciphertext (or integrity journal + ciphertext). A host rewrite of bytes that *look* like a LUKS header at the start of `P_data` MUST be ignored.

Default LUKS2 parameters when `profile` includes `crypt`:

| Parameter | Value |
|-----------|--------|
| Cipher | `aes-xts-plain64` |
| Key size | 512-bit XTS |
| Integrity | `hmac-sha256` iff `crypt+integrity` |
| Sector size | 4096 |
| `subsystem` | `cdf-v1` |
| `label` | volume UUID in text form |

---

## 5. Profiles

| `profile` | Confidentiality | Integrity | Typical use |
|-----------|-----------------|-----------|-------------|
| `plain` | none | none | Non-secret scratch; still GPT-discoverable |
| `crypt+integrity` | LUKS2 | `dm-integrity` | Default confidential RW volume |
| `verity` | none | read-only tree | Immutable public image |
| `verity+crypt` | LUKS2 then verity | both | Immutable secret image |

**Encryption implies integrity.** There is no encryption-only profile, and an implementation MUST NOT add one. `dm-crypt` with AES-XTS is length-preserving and unauthenticated: a host that can write the device can alter plaintext at 16-byte-block granularity, blindly but repeatably, and the guest accepts the result with no signal. Since the whole premise of this format is a host that can write, an encryption-only profile would be a mode that looks protected and is not. The implication is one-way: `verity` gives integrity without confidentiality, which is a legitimate configuration.

Consequently a conforming opener MUST refuse a Contract whose `profile` names encryption without integrity, even if the MAC verifies — the value is outside the enum, and treating it as `crypt+integrity` would silently change what was authenticated.

`flags.AUTHENTICATED` MUST be set for every profile except `plain`: `verity` needs an authenticated root hash even though it holds no secret. `plain` has no key, therefore no MAC, and an opener MUST treat it as an ordinary unprotected partition.

For every authenticated profile the opener MUST take `profile` from the **MAC-verified** Contract. The same bytes read before verification are a display value only.

---

## 6. Keys

### 6.1 Hierarchy

```
K_root     <-- KBS after attestation, or CVM-owner key file
              bound to volume_uuid; MUST NOT be shared across volumes

K_mac      = HKDF-SHA-256(K_root, salt=volume_uuid, info="cdf-v1/meta-mac")
K_luks     = HKDF-SHA-256(K_root, salt=volume_uuid, info="cdf-v1/luks")

HMAC       uses K_mac
LUKS2      passphrase / key-file is K_luks   -- not K_root raw, not a key parsed from the header
```

`K_mac` is **never** derived from a volume key recovered by parsing an on-disk LUKS header. That order would trust the host-controlled header in order to authenticate it.

`K_luks` is a uniformly random 256-bit key, not a human passphrase. Two consequences: publishing the LUKS2 header in cleartext does not enable an offline guessing attack on the keyslot, and keyslot KDF hardening is not load-bearing (it protects low-entropy passphrases, which CDF does not use).

### 6.2 Key acquisition is out of band

**The image does not say where its key comes from.** It carries the volume's *identity* (`volume_uuid`); the deployment carries the *locator*. An opener obtains `K_root` from a trusted channel, keyed by `volume_uuid`:

| Channel | Example |
|---------|---------|
| Measured guest config | CoCo Init-Data (`cdh.toml`), SMBIOS OEM strings carrying `initdata.toml` (the [`kbs` token plugin](https://github.com/MatiasVara/libcryptsetup-token-kbs) prefers this over on-disk fields) |
| Measured orchestration policy | A Trustee manifest reference authorized by measured Agent policy ([Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638)) |
| CoCo storage plumbing | `kbs-key-id` in the StorageClass / CSI path |
| CVM owner | A key file or KMS reference supplied by the owner at boot |

Three reasons the locator does not belong on the image:

1. **It cannot be authenticated in time.** The locator is needed *before* any key is available, so it is necessarily consumed while still unverified. Putting it on an attacker-writable medium means the opener must cross-check it against a trusted list anyway — at which point the trusted list is the real source and the on-image copy only adds a mismatch case to handle.
2. **It is not needed to interpret the bytes.** Cipher, layout, integrity profile, and verity root are properties of the image. Which key service holds the key is a property of the environment.
3. **It couples the image to one deployment.** Re-homing the key service, or moving the image between environments, would otherwise require rewriting authenticated metadata and burning a generation for a routing change.

The volume always receives **key bytes** (`K_luks`), never a URI. The KBS SHOULD release `K_root` only when the request matches `(attestation, volume_uuid)`. `K_root` is a per-volume secret and is **not** rotated by this format on open; rotation is an administrative operation (§8.4).

Trust of opener inputs:

| Channel | What it supplies | Trust |
|---------|------------------|--------|
| Superblock + Contract, before MAC | `volume_uuid` to look the key up; everything else for display | **Untrusted** |
| Measured CoCo / CVM config | locator, or `K_root` directly | Trusted if measured |
| KBS | `K_root` | Trusted after attestation |
| Superblock + Contract, after MAC | the whole open contract | Trusted |

Reading `volume_uuid` before verification is safe: if the host substitutes it, the opener requests the wrong key, derives the wrong `K_mac`, and the MAC fails — because `volume_uuid` is inside the MAC input. The failure mode is a refused open, not a wrong open. A deployment that already knows the expected `volume_uuid` SHOULD compare it and refuse early.

---

## 7. Algorithms

| Use | Algorithm | Notes |
|-----|-----------|--------|
| Meta authentication | **HMAC-SHA-256** | 32-byte tag in the superblock; no nonce; computed rarely (only on Contract rewrites) |
| KDF | HKDF-SHA-256 | RFC 5869 |
| LUKS2 payload | `aes-xts-plain64` | Allow-list; reject any other cipher including null |
| RW integrity | `hmac-sha256` via LUKS2 `--integrity` | Journal on |
| RO integrity | `sha256` dm-verity | Root hash in Contract |

A conforming opener MUST refuse `Contract.data.cipher` outside `{aes-xts-plain64}` and MUST refuse a LUKS2 header whose JSON segment encryption is not exactly that value (defense in depth after the MAC).

### 7.1 Why a MAC and not AEAD

The threat is a host that **rewrites** the open contract, not a host that **reads** it. Every field in the Contract is either public by nature or already public elsewhere:

| Field | Secret? | Why not |
|-------|---------|---------|
| `profile`, `cipher`, `integrity` | no | Inferable from partition sizes and from the `dm-integrity` / verity geometry the host can already see |
| `verity.root_hash_hex`, `salt_hex` | no | A verity root hash is a public commitment; other systems sign and publish it |
| `luks2_header_b64` | no | A LUKS2 header is stored in the clear on every ordinary LUKS device. Its keyslots hold the volume key wrapped under `K_luks`, which is a full-entropy 256-bit key (§6.1), so exposure gives no offline attack |
| `volume_uuid`, `meta_generation` | no | Identity and version, both in the cleartext superblock anyway |

Given that, encryption buys nothing and costs something:

- **No nonce to manage.** AES-GCM needs a unique nonce per seal under the same key, and nonce reuse loses the authentication key, not just plaintext. HMAC has no such failure mode. The contract is resealed on every rekey and profile change, by different tools, possibly after a crash — precisely the conditions under which nonce discipline goes wrong.
- **One object instead of two.** With an encrypted contract the format needs a second cleartext structure so that a volume can be recognised before any key exists, plus a rule that the two must agree. Authenticating a cleartext contract removes the duplicate and the agreement rule.
- **Recoverable and inspectable.** `blkid`-style tooling, an operator debugging a failed mount, and a forensic reader can all see what the volume claims to be without holding a key, while still being unable to change it.
- **It matches the precedent.** tdx-init and CDH #1648 both MAC a cleartext LUKS2 header; neither encrypts it.

AEAD would be the right choice if the contract had to carry a secret — a wrapped key, a token, a policy the host should not read. It does not, and §4.2 makes that a normative constraint so the assumption cannot quietly rot.

What the MAC must get right, since these are the ways a MAC-only design fails: it MUST cover the binary superblock as well as the JSON (otherwise offsets, `flags`, and `volume_uuid` are malleable), the covered length MUST itself be authenticated (otherwise the boundary between the two inputs is ambiguous), and the tag comparison MUST be constant-time. §4.2 specifies all three.

---

## 8. Procedures

Notation: `Zero(P)` means every byte of partition `P` is 0x00.

### 8.1 Identify

1. Parse GPT. Require `P_meta` with type `4ca88ee1-60f4-4b6b-899e-dae026ced8e8`.
2. Read Superblock. `magic` MUST be `CDF1\0\0\0\0`, `version_major` MUST be 1.
3. Read `volume_uuid`. Everything else read at this stage is untrusted.

### 8.2 First format (init)

Allowed only when **all** of: `Zero(P_meta)`, `Zero(P_data)`, and `[P_verity]` absent or zero; **and** policy permits init (CoCo: measured `readWrite` + empty source; CVM: owner action).

1. Allocate `volume_uuid`, set `meta_generation := 1`.
2. Obtain `K_root` (§6.2). Derive `K_mac`, `K_luks`.
3. If the profile uses LUKS: `luksFormat` with detached header into a tmpfs file; passphrase `K_luks`; parameters from §4.3. Do not write a LUKS header onto `P_data`.
4. If the profile uses verity: build the hash tree onto `P_verity`; put the root hash into the Contract.
5. Serialize the Contract, fill the Superblock (`INITIALIZED|AUTHENTICATED|…`, `contract_size`, `mac := 0`), compute the MAC over both, store it in `mac`.
6. Write Contract, then Superblock last, so a crash leaves an image that fails identification rather than one that verifies against half a contract.
7. Open via §8.3 and hand the mapper to the filesystem / workload.

### 8.3 Open (reopen)

The whole point of the ordering below is that **no byte from the image influences a device-mapper decision before the MAC verifies**.

1. **Identify** (§8.1). Obtain `volume_uuid` from the Superblock.
2. **Decide what is expected**, from the deployment and not from the image: is this volume required to be authenticated, and is it read-only or read-write? If the deployment expects an authenticated volume and `flags.AUTHENTICATED == 0`, **refuse**.
3. If the Superblock is zero or the magic is unknown:
   - if the §8.2 init conditions hold → format;
   - else **refuse**. Never format a non-zero or half-written image.
4. **Get the key.** Resolve the locator from the trusted channel (§6.2), attest, receive `K_root`. Derive `K_mac` and `K_luks`.
5. **Verify.** Recompute `HMAC-SHA-256(K_mac, Superblock[mac := 0] || Contract[0 .. contract_size))` and compare in constant time.
   - **On mismatch: refuse.** Do not format, do not fall back to `plain`, do not open `P_data` without a header, do not retry with a different interpretation.
6. **Cross-check.** `Contract.volume_uuid` and `Contract.meta_generation` MUST equal the Superblock; `Contract.profile` MUST be one the deployment allows; `Contract.data.cipher` MUST be `aes-xts-plain64`.
7. **Activate**, in the order implied by `profile`:
   - verity first if present: `dm-verity` with the root hash from the Contract, never from `P_verity`'s own superblock;
   - then LUKS: write `luks2_header_b64` to a tmpfs file `H_ram`, allow-list its JSON, `cryptsetup open --header H_ram`.
8. **Mount** with the access mode from step 2, and return the mapper.

Worked example for `crypt+integrity`, with `K_root` already in guest memory:

```sh
# 1-3. identify and read, still untrusted
sb=$(dd if=/dev/sda1 bs=4096 count=1 status=none | xxd -p)      # P_meta superblock
uuid=$(cdf-tool show-uuid /dev/sda1)                            # only field used pre-verify

# 4. key from a trusted channel, keyed by identity
K_root=$(cdf-tool get-key --uuid "$uuid")                       # attestation happens here
K_mac=$( hkdf-sha256 "$K_root" --salt "$uuid" --info cdf-v1/meta-mac)
K_luks=$(hkdf-sha256 "$K_root" --salt "$uuid" --info cdf-v1/luks)

# 5. verify before anything else; non-zero exit means stop, not fall back
cdf-tool verify /dev/sda1 --key "$K_mac" || exit 1

# 6-7. only now is the contract usable
cdf-tool extract-header /dev/sda1 > /run/cdf/H_ram             # tmpfs, guest RAM
cdf-tool check-header /run/cdf/H_ram --allow-cipher aes-xts-plain64 || exit 1
printf '%s' "$K_luks" | cryptsetup open --type luks2 \
    --header /run/cdf/H_ram --key-file - /dev/sda2 cdf-data

# 8.
mount -o ro /dev/mapper/cdf-data /mnt/data                     # rw only if step 2 said so
```

Note what is absent: no `cryptsetup open` against `/dev/sda2` without `--header`, no `luksFormat` anywhere on this path, and no branch that treats a verification failure as "this must be a new disk".

### 8.4 Rewrite the Contract

Any authorized rewrite (rekey, profile change, new verity root) MUST increment `meta_generation`, recompute the MAC, and write Contract before Superblock, as in §8.2. A Contract from an earlier generation then fails verification against the current Superblock, so a host cannot revert a completed rekey or profile change by restoring the old bytes.

Rewriting is not part of the open path: opening a volume never changes `meta_generation`, so no writable state is required at the key broker.

---

## 9. How CoCo and a bare CVM consume the same image

```
                         +------------------+
                         | CDF image (GPT)  |
                         +------------------+
                                  |
              discover P_meta (type UUID) — no plugin required
                                  |
              +-------------------+-------------------+
              |                                       |
     CoCo: measured policy                    CVM: owner / cloud-init
     supplies the key locator                 supplies the locator or K_root
              |                                       |
              +-------------------+-------------------+
                                  |
                    look up by volume_uuid from the image
                                  |
                         KBS (optional) --> K_root
                                  |
                         verify MAC --> Contract is usable
                                  |
                         cryptsetup --header H_ram
```

The locator comes from whichever of those two paths exists; the image never provides it (§6.2). CoCo MAY pass `kbs-key-id` in CSI metadata — that is untrusted transport, so it is acceptable only because the MAC afterwards proves the key was the right one: a wrong or attacker-chosen key yields a verification failure and a refused open. A CVM holding only the disk needs an owner-supplied locator or `K_root`; the disk alone is not sufficient, by design.

---

## 10. Residual attacks (not solved by the MAC alone)

| Attack | Result | Extra control |
|--------|--------|----------------|
| Rewrite any Contract or Superblock field without `K_mac` | MAC fails; open refused | §8.3 fail-closed |
| Clear `flags.AUTHENTICATED` to present a `plain` volume | Refused | Expected mode comes from the deployment, not the image (§4.2 step 2 of §8.3) |
| Rewrite live bytes at the start of `P_data` | Ignored (`--header H_ram`) | Detached payload |
| Restore only an **old** Contract under this `K_mac` | Refused | `meta_generation` inside the MAC input (§8.4) |
| Substitute `volume_uuid` to redirect the key lookup | MAC fails; open refused | `volume_uuid` inside the MAC input (§6.2) |
| Swap a **whole** other CDF disk minted under the same `K_root` | MAC can succeed | One `K_root` per `volume_uuid`; deployment checks the expected UUID |
| Restore an older consistent copy of the **whole** image | Accepted; tags and MAC both verify | **Out of scope** (§3). Needs an external freshness anchor |
| Read the Contract | Succeeds by design | Nothing in it is secret (§7.1); the payload stays encrypted |
| Flip ciphertext sectors | Detected in **every** confidential profile | Guaranteed by §5: no profile encrypts without integrity. Undetected only in `plain`, which claims nothing |

---

## 11. Conformance checklist

An implementation is CDF v1 conformant if it:

1. Uses the GPT type UUIDs in §4.1.
2. Authenticates the Contract with HMAC-SHA-256 under HKDF-derived `K_mac`, over the zero-`mac` Superblock followed by exactly `contract_size` Contract bytes, compared in constant time.
3. Uses no field of the Contract before that comparison succeeds, apart from `volume_uuid` for the key lookup.
4. Opens LUKS2 only via `--header` from the verified Contract.
5. Refuses null / non-allow-listed ciphers even after the MAC verifies, and refuses any `profile` that encrypts without integrity (§5).
6. Never formats except under §8.2, and never in response to a verification failure.
7. Takes "must be authenticated" and "read-only vs read-write" from the deployment, not from the image.
8. Stores no key locator on the image (§6.2).
9. Puts no secret in the Contract (§4.2).
10. Increments `meta_generation` on every Contract rewrite and never decreases it.

---

## 12. References

- [LUKS2 On-Disk Format Specification](https://gitlab.com/cryptsetup/LUKS2-docs)
- [dm-integrity](https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html)
- [dm-verity](https://docs.kernel.org/admin-guide/device-mapper/verity.html)
- [UEFI GPT](https://uefi.org/specs/UEFI/2.10/05_GUID_Partition_Table_Format.html)
- [RFC 5869 HKDF](https://www.rfc-editor.org/rfc/rfc5869)
- [Trail of Bits: LUKS2 in confidential VMs](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)
- [flashbots/tdx-init](https://github.com/flashbots/tdx-init)
- [guest-components #1648](https://github.com/confidential-containers/guest-components/pull/1648), [#1663](https://github.com/confidential-containers/guest-components/pull/1663)
- [systemd Home Directories](https://systemd.io/HOME_DIRECTORY/)
- [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs)
- Survey in this repo: [investigate/README.md](../investigate/README.md)
