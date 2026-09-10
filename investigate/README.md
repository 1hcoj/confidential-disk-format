# Survey: on-disk metadata for confidential and integrity-protected volumes

**Purpose.** Decide whether an existing open format already satisfies this repository's goals, or whether a new volume image format must be specified here.

**Scope.** Public, checkable on-disk formats and conventions used by Linux disk encryption, confidential VMs (CVMs), and Confidential Containers (CoCo). Platform-only cloud contracts are recorded only when they illustrate a capability we want, not as portable formats.

**Method.** For each candidate: (1) state the on-disk layout in a uniform formal notation, (2) record confidentiality / integrity / self-description / key-release properties, (3) measure the gap to the requirements in the [repository README](../README.md).

**Date.** 2026-09-10.

**中文译本 / Chinese translation:** [README.zh.md](README.zh.md)

---

## 1. Requirements under survey

### 1.1 Protection modes the format must be able to express

The repository goal "optional confidentiality and integrity" is really a choice of device-mapper stack. Spelling the modes out first, because which modes are in scope decides whether a new partition layout is needed at all (§1.3).

| Mode | Device-mapper stack | Mutability | Extra on-disk area | Confidentiality | Integrity |
|------|---------------------|------------|--------------------|-----------------|-----------|
| `plain` | none | RW | none | no | no |
| `crypt+integrity` | `dm-integrity` → `dm-crypt` | RW | none (tags/journal are in-band, `cryptsetup --integrity`) | yes | **RW integrity** |
| `verity` | `dm-verity` | **RO** | **hash tree** (separate partition, or a hash offset on the same device) | no | **RO integrity** |
| `verity+crypt` | `dm-crypt` → `dm-verity` | **RO** | same as `verity` | yes | **RO integrity** |

**Encryption implies integrity: there is deliberately no encryption-only mode.** `dm-crypt` with AES-XTS is length-preserving and unauthenticated, so a host that can write the device can alter plaintext — per 16-byte block, blindly but repeatably — and the guest accepts the result with no signal at all. In a threat model whose entire premise is a host that can write, offering confidentiality without integrity would be offering a mode that looks protected and is not. So every confidential mode here is paired with integrity: RW pairs with `dm-integrity` tags, RO with `dm-verity`. The implication is one-way — integrity *without* confidentiality stays available as `verity`, for an immutable public image.

**Both integrity kinds are in scope, and they are not interchangeable.** RW integrity is entirely in-band: `cryptsetup --integrity` puts tags and a journal inside the same device, so it needs no new on-disk area and no new metadata. RO integrity needs two things LUKS2 has nowhere to put: a **second extent** for the hash tree, and an **authenticated root hash** (plus salt and block geometry) that must arrive from outside the tree it validates. That asymmetry decides the layout question (§1.3), and it is what disqualifies the otherwise-closest surveyed class (§6.3).

### 1.2 Requirements

| ID | Requirement | Meaning for the on-disk format |
|----|-------------|-------------------------------|
| R1 | Optional confidentiality and integrity | The format MUST record which mode of §1.1 applies, and that record MUST be the one the opener acts on. |
| R2 | Versatility | The same image MUST be openable by a CoCo guest (CDH / Trustee) **and** by a standalone CVM that does not run Kata. Opening MUST NOT require a sidecar file that exists on only one orchestrator. |
| R3 | Self-contained | Everything needed to *interpret* the image (mode, layout, cipher, integrity profile, verity root) MUST be on the image. What MUST NOT be on the image: secrets, and the **key locator**. The locator is an environment property, is needed before any key is available (so it can never be authenticated in time), and would pin one image to one deployment; it is supplied by the trusted channel instead. The image carries the volume's identity, and the deployment maps that identity to a key. |
| R4 | Out-of-box | A Linux guest with `cryptsetup` / device-mapper SHOULD open the data plane with no new userspace crypto stack. |
| R5 | Authentic open contract | **Added after the survey.** The adversary can rewrite every byte, so the record in R1/R3 MUST be authenticated (MAC / AEAD / measured) with a key the host does not hold, and the open path MUST consume the authenticated copy. Stock LUKS2 does not meet this ([§4.1](#41-luks2), [Trail of Bits](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)). |

R5 was implicit in the original goal list. It is called out because it, not encryption, is what stock formats fail.

**Explicitly out of scope: freshness of the whole device.** An earlier draft of this list carried an anti-replay requirement. It is dropped, because no on-disk format can satisfy it. The host can restore an older, internally consistent copy of the image, and every check that lives on the disk — integrity tags, header MAC, a generation counter — passes, since the adversary rolled back the evidence together with the data. Detecting it needs mutable state *outside* the disk (a key broker that remembers the newest generation per volume, or a guest-visible monotonic counter), so it is a property of the deployment, not of the layout. [Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) reaches the same conclusion and lists whole-snapshot freshness as a non-goal: "neither the disk nor Trustee records which authenticated state is newest". This survey therefore records where each project stands on replay as context, but does not score it as a requirement.

### 1.3 Which requirement forces which structure

| If the scope is… | Then structurally you need… | Why |
|------------------|------------------------------|-----|
| `crypt+integrity` only (no RO integrity) | **nothing beyond LUKS2** | Both stacks are in-band: the LUKS2 header describes the cipher, and `--integrity` puts tags + journal inside the same device. Any extra metadata fits in `tokens`. |
| …plus R5 | LUKS2 **plus one authenticated anchor** | The anchor cannot be a plain token: a MAC over the header cannot live inside the header it covers. Options: a small MAC region on the device (tdx-init, CDH #1648), a detached header held off the untrusted device, or a header digest bound into the KBS release policy. None of these requires a partition table. |
| …plus `verity` / `verity+crypt` | a **hash area**, and in practice a **partition map** | A verity hash tree is a second object with its own size and root hash. It can live at a hash offset on the same device (`veritysetup --hash-offset`), but then offsets become private convention. GPT type UUIDs make the roles discoverable, which is the R2/R3 argument for `GPT(meta, [verity], data)`. |

So the layout question reduces to: is read-only integrity in scope, and does the authenticated anchor need to be discoverable without private offsets? Both answers are "yes" for this repo, which is why §7 lands on a partition-based envelope rather than on tokens alone.

### 1.4 Baseline design intent

Working design intent used as the comparison baseline (not yet a spec):

```
TargetImage  ::=  GPT( P_meta [, P_verity], P_data )

P_meta       ::=  Superblock · OpenParams · MAC_K(Superblock · OpenParams)
P_verity     ::=  VeritySuperblock    ·  HashTree          -- optional, read-only
P_data       ::=  LUKS2[ + dm-integrity ]                  -- read-write payload

P_meta is cleartext; nothing in it is secret, so it is authenticated, not encrypted.
It carries the volume identity but no key locator; the locator comes from the
trusted channel, and the MAC key is released only after attestation.
CDH (or an equivalent CVM agent) can open the image without Kata.
```

---

## 2. Notation

All layouts below use this grammar. Concatenation is written `A · B`. Optional parts are `[A]`. Repetition is `A+` or `A*`. Sizes are in bytes unless noted. Offsets are from the start of the named object.

```
Device            ::=  bytes[0 .. |Device|)

GPT               ::=  ProtectiveMBR · GPTHeader · PartitionEntryArray
                       · Partition+ · BackupPartitionEntryArray · BackupGPTHeader

Partition(type, label) ::=  bytes[FirstLBA .. LastLBA]

LUKS2             ::=  H_pri · J_pri · H_sec · J_sec · KeyslotArea · [Pad] · Payload

H_*               ::=  luks2_hdr_disk          -- 4096 B binary header
J_*               ::=  JSON_NUL · 0x00*        -- |J| = H.hdr_size - 4096
KeyslotArea       ::=  WrappedVolumeKey*
Payload           ::=  ciphertext[ segment.offset .. )

JSON              ::=  { config, keyslots, digests, segments, tokens }

IntegrityDevice   ::=  Reserved · Superblock_4KiB · Journal · (TagRun · DataRun)+
VerityDevice      ::=  [VeritySB_512] · PadToHashBlock · HashTree
```

Security adjectives used in the tables:

| Term | Definition |
|------|------------|
| **Cleartext metadata** | Readable and mutable by the storage adversary. |
| **Checksummed metadata** | Detects accidental corruption, not a malicious rewrite (LUKS2 SHA-256 header csum). |
| **MAC / AEAD metadata** | Detects malicious rewrite, given a key not held by the storage adversary. |
| **Measured metadata** | Hash is bound into attestation / PCR / RTMR before the key is released. |
| **Detached header** | Open path reads header bytes from trusted RAM, not from the live untrusted device (closes TOCTOU). |

The LUKS2 specification itself states that its checksum and redundancy cover *random* corruption only, not intentional modification ([LUKS2 spec §1.2](https://gitlab.com/cryptsetup/LUKS2-docs)). That sentence is the reason almost every CVM project below adds an envelope around LUKS2.

---

## 3. Conclusions

| Observation | Detail |
|-------------|--------|
| There is **no** industry-standard “confidential persistent volume” image format | Projects compose **LUKS2** plus private conventions. |
| LUKS2 **is** on-disk metadata | Binary header ×2, JSON (`keyslots`, `segments`, `tokens`, …), keyslot area, payload. Tokens are the official extension point for “how to get the key”. |
| LUKS2 header is **not** authentic against a host / storage adversary | Default AES-XTS is length-preserving and unauthenticated. A host can rewrite JSON (cipher, segment, even `cipher_null-ecb`). See [Trail of Bits, 2025-10-30](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/). |
| CVM consensus after that research | (1) copy header to guest RAM and `cryptsetup open --header`, (2) **MAC the header** or **allow-list the JSON**, optionally (3) **measure** a sanitized header into attestation. |
| Closest “self-describing portable image” | [systemd-homed](https://systemd.io/HOME_DIRECTORY/): `GPT · LUKS2 · token(encrypted user record)`. Closest “attestation + KBS on disk”: [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs) (`tokens[i].type = "kbs"`). Neither provides a GPT meta + verity + authenticated-profile image. |
| Closest productized CoCo/CVM data-volume stack | [cryptpilot](https://github.com/openanolis/cryptpilot): LUKS2 + optional `dm-integrity` + Trustee/KBS. **Key URI lives in guest TOML**, not in an authenticated on-image object. FDE verity root hash lives in **initrd**, not in a disk meta partition. |
| CoCo persistent storage is still converging | CAA CSI passes `encrypt-type` / `kbs-key-id` in **host** `mountInfo.json`. [guest-components #1648](https://github.com/confidential-containers/guest-components/pull/1648) / [#1663](https://github.com/confidential-containers/guest-components/pull/1663) add an authenticated LUKS envelope + integrity. [Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) proposes putting the profile in a **Trustee manifest**, not on the disk. |
| A new GPT three-partition image is **not** forced by missing encryption | Confidentiality alone needs nothing new. What forces it is **read-only integrity** (a hash tree is a second extent, and its root hash must be recorded somewhere authenticated) together with (R2)+(R3): those parameters must travel **with the image**, independently of Kata, CSI, and guest config. See §6.3. No surveyed project publishes that as a stable spec. |

**Recommendation for the next step in this repo.** Treat LUKS2 + `dm-integrity` + `dm-verity` as the **data-plane** primitives (R4). Specify a **thin, authenticated envelope** (either a GPT `P_meta` or a CDH-style header MAC region) that carries the open profile (R2, R3, R5), and keep the key locator off the image entirely (R3). Authentication is enough: the envelope holds no secret, so a MAC beats an AEAD here. Do not attempt device-level freshness on the disk, and do not invent a new cipher suite or a cryptsetup-incompatible payload.

That envelope is specified as **CDF v1** in [design/README.md](../design/README.md).

---

## 4. Building-block formats

These are not competing products. Every later project is a composition of one or more of them.

### 4.1 LUKS2

| | |
|---|---|
| **Name** | Linux Unified Key Setup 2 |
| **Spec** | [LUKS2 On-Disk Format Specification v1.1.4 (2025-06-16)](https://gitlab.com/cryptsetup/LUKS2-docs) · [PDF mirror](https://fossies.org/linux/cryptsetup/docs/on-disk-format-luks2.pdf) · [cryptsetup](https://gitlab.com/cryptsetup/cryptsetup) |
| **Role** | De-facto Linux encrypted-block container. Payload of almost every project below. |

**On-disk format.**

```
LUKS2_device ::=  H_pri · J_pri · H_sec · J_sec · KeyslotArea · [Pad] · Payload

H_pri            at byte 0
                 magic    = "LUKS" || 0xBA 0xBE
                 version  = 2
                 hdr_size = |H_pri · J_pri|          -- 16 KiB, 32 KiB, …, 4 MiB
                 seqid    : monotonically increased on each header write
                 csum     : SHA-256(H_pri[csum:=0] · J_pri)   -- integrity vs accident only
                 uuid, label, subsystem, salt, hdr_offset

J_pri            at byte 4096
                 |J_pri|  = hdr_size - 4096
                 contents = JSON as a NUL-terminated C string, remainder 0x00

H_sec            at offset hdr_size
                 magic    = "SKUL" || 0xBA 0xBE
                 otherwise same schema as H_pri; salt MUST differ

J_sec            replica of J_pri (same functional metadata)

KeyslotArea      allocated on demand; NOT replicated
Payload          at segments["0"].offset  (typically 16 MiB)
```

Allowed secondary-header offsets (`hdr_size`): 16 KiB, 32 KiB, 64 KiB, 128 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 4 MiB.

```
+------------+------------------+------------+------------------+---------------+--------+
| H_pri 4KiB | J_pri            | H_sec 4KiB | J_sec            | KeyslotArea   | Data   |
+------------+------------------+------------+------------------+---------------+--------+
0            4096               hdr_size                        keyslots.offset  segment.offset
```

**JSON schema (mandatory top-level objects).**

```
JSON = {
  config   : { json_size, keyslots_size, flags? },
  keyslots : { id -> Keyslot },     -- wrapped volume key + KDF + AF stripes
  digests  : { id -> Digest },      -- verifies a recovered volume key
  segments : { id -> Segment },     -- ciphertext region: offset, size, encryption, sector_size, integrity?
  tokens   : { id -> Token }        -- "how to obtain a passphrase" (opaque JSON + type)
}

Keyslot  = { type: "luks2", key_size, af, area, kdf }
Segment  = { type: "crypt" | "hw-opal" | "hw-opal-crypt",
             offset, size, encryption, sector_size, integrity? }
Token    = { type: string, keyslots: [id], ...type-specific fields }
```

**Optional integrity (authenticated encryption).** If `segments[i].integrity` is set, cryptsetup builds:

```
/dev/X  --dm-integrity-->  /dev/mapper/X_dif  --dm-crypt-->  /dev/mapper/X
```

Typical production choice: `--cipher aes-xts-plain64 --integrity hmac-sha256`. Native AEAD (`aes-gcm-random`, `aegis128-random`) is still experimental in cryptsetup. The GCM 96-bit nonce is **not** recommended for FDE.

**Capabilities.** Confidentiality (`dm-crypt`). Optional per-sector integrity. Multiple keyslots. Detached header (`cryptsetup --header`). Token extension point. OPAL hardware-encryption segments (`hw-opal`, `hw-opal-crypt`) since cryptsetup 2.7.

**Gap vs R1–R5.** Header is cleartext + checksummed, not MAC'd (R5). No first-class “attest, then fetch key from KBS” object in the spec. Tokens can *name* an external key source, but nothing authenticates the token against the host.

---

### 4.2 dm-integrity

| | |
|---|---|
| **Name** | Linux device-mapper integrity target |
| **Spec** | [kernel docs: dm-integrity](https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html) |

**On-disk format.**

```
IntegrityDevice ::=  ReservedSectors · Superblock · Journal · Interleaved+

Superblock      ::=  4096 B
                     magic, version,
                     log2(interleave_sectors),
                     tag_size,
                     journal_section_count,
                     provided_data_sectors

Journal         ::=  JournalSection+
JournalSection  ::=  Metadata_4KiB · DataArea
                     Metadata contains {logical_sector, tail_8B, tag}*
                     plus 8 B MAC over sector numbers and a commit id
                     (atomic journal, not a cryptographic MAC of user data
                      unless an integrity key is configured)

Interleaved     ::=  TagRun · DataRun
TagRun          ::=  tags[n], |TagRun| ≥ 4 KiB
DataRun         ::=  data_sectors[2^log2_interleave]
```

Standalone `dm-integrity` (no LUKS) can use CRC for silent-corruption detection. That is **not** authentication. Cryptographic mode (`hmac-sha256`, or AEAD via LUKS2) requires a key.

**Capabilities.** Per-sector tamper evidence for *mutable* data. Journal gives atomicity of data+tag. Does **not** by itself prevent replay of an older, internally consistent snapshot (the host replaces the whole device).

**Note.** Integrity ≠ freshness. This is a real limitation, but per §1.2 it is out of scope for the format: closing it requires state outside the disk.

---

### 4.3 dm-verity

| | |
|---|---|
| **Name** | Linux device-mapper verity target |
| **Spec** | [kernel docs: dm-verity](https://docs.kernel.org/admin-guide/device-mapper/verity.html) · [cryptsetup DMVerity wiki](https://gitlab.com/cryptsetup/cryptsetup/-/wikis/DMVerity) |

**On-disk format** (userspace convention; the kernel does **not** parse the superblock — it only consumes hash blocks + a root hash supplied from a trusted channel):

```
VerityDevice ::=  [SB] · PadTo(hash_block_size) · HashTree

SB ::= 512 B
     signature        = "verity\0\0"
     version          = 1
     hash_type        = 0 (Chrome OS) | 1 (current)
     uuid
     algorithm[32]    -- e.g. "sha256"
     data_block_size, hash_block_size
     data_blocks
     salt_size, salt[256]

HashTree ::= depth-first, root first, increasing index
             leaf  = H(salt || data_block)     -- type 1: salt prepended
             node  = H(salt || child_hashes)
```

**Capabilities.** Read-only integrity / authenticity of a static tree, **iff** the root hash is trusted (initrd, measured config, or an authenticated meta partition). Natural fit for immutable images.

**Gap vs R3.** The root hash is *not* self-authenticating. Someone must carry it in a trusted channel. cryptpilot FDE puts it in initrd; the target design would put it in `P_meta`.

---

### 4.4 GPT (discovery layer)

A GUID Partition Table is not a crypto format. It is the usual way to make several of the objects above *discoverable* on one image.

```
Image ::= GPT(
            P_i  where P_i = LUKS2 | VerityDevice | IntegrityDevice | Filesystem | Opaque
          )
```

Discovery is by **type UUID** and **partition label**. systemd-homed is the cleanest precedent for “this GPT type means this LUKS+token contract” (Class M, §5.4). A new confidential-volume format would mint its own type UUIDs for `P_meta` / `P_verity` / `P_data`.

---

## 5. Formats, grouped by class

The useful axis is not the project but **where the open contract lives and whether it is authenticated**. Six classes cover everything surveyed. Projects are members of a class; a few appear twice because they combine classes.

| Class | Shape | What it adds to LUKS2 | Members |
|-------|-------|-----------------------|---------|
| **L** | `Device ::= LUKS2`; contract in the header (`tokens`) | nothing | Clevis/Tang, systemd-cryptenroll, libcryptsetup-token-kbs, systemd-homed, Constellation, tdx-init (SSH key) |
| **C** | `Device ::= LUKS2`; contract **not** on the image | nothing | cryptpilot-crypt, Contrast, Cosmian, dstack, Oasis, Secret Network, CAA CSI, Kata #13638 |
| **E** | `Device ::= Envelope · Payload`; authenticated region on the same device | MAC, detached header, crash state | tdx-init v0.2.0, CDH #1648/#1663, Fortanix Salmiac |
| **M** | `Image ::= GPT(roles…)`; roles in a partition table | discovery, and room for a verity tree | systemd-homed image, cryptpilot FDE, **CDF v1 (this repo)** |
| **P** | platform contract, not a portable image | n/a | Azure, GCP, AWS / Nitro, IBM SE, qcow2+LUKS |
| **X** | non-LUKS whole-disk format | n/a — different data plane | VeraCrypt, BitLocker, TCG Opal, ZFS / fscrypt / ocicrypt |

Classes L and C put the **same bytes on disk**. They differ only in whether the open profile travels with the image, which is invisible to `blkid`. Note that Class L's `tokens` typically also carry a key locator; under R3 as stated in §1.2 that part is surplus rather than required, and being unauthenticated it is a liability.

### 5.1 Class L — stock LUKS2, open contract inside the LUKS2 header

**Format.** Unmodified LUKS2 (§4.1). The contract is one or more `tokens[i]`; cryptsetup stores the JSON verbatim and ignores types it does not know.

```
Device     ::=  LUKS2
tokens[i]  ::=  { type: <string>, keyslots: [k], ...type-specific fields }

-- the JSON area is cleartext; the only protection is an unkeyed SHA-256 csum,
-- so any host rewrite that keeps the csum consistent is accepted
```

| Member | `tokens[i].type` | What the token holds | Note |
|--------|------------------|----------------------|------|
| [Clevis / Tang](https://github.com/latchset/clevis) | `clevis` | Jose object per pin: Tang ads, TPM2 sealed blob, SSS threshold | network / TPM unlock; assumes a notary, not a TEE |
| [systemd-cryptenroll](https://www.freedesktop.org/software/systemd/man/latest/systemd-cryptenroll.html) | `systemd-tpm2`, `systemd-fido2`, `systemd-pkcs11`, `systemd-recovery` | sealed blob or public key + PCR policy | LUKS2-only by design: the JSON token area *is* the storage |
| [systemd-homed](https://systemd.io/HOME_DIRECTORY/) | `systemd-homed` | `record` = Base64(Enc_VK(user record)), `iv` | encrypted under the volume key with its own IV; also Class M |
| [Constellation](https://docs.edgeless.systems/constellation/architecture/encrypted-storage) | `constellation-state-disk` | `diskIsInitialized`, … | DEK from the cluster KeyService; after [GHSA-hq76-6gh2-5g4q](https://github.com/edgelesssys/constellation/security/advisories/GHSA-hq76-6gh2-5g4q) it opens via detached header + JSON allow-list |
| [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs) | `kbs` | KBS URL + resource path, or a pointer to SMBIOS `initdata.toml` | closest existing “attest, then unlock” on-disk contract; requires the plugin |
| [tdx-init](https://github.com/flashbots/tdx-init) | header metadata, not a token | searcher SSH key | makes first boot self-describing; its MAC sector is Class E |

**Properties.** R3 holds for the crypt parameters, which live in the LUKS2 header; it has nowhere to record a verity root. R4 is perfect: any `cryptsetup` opens it. R1 is recorded in `segments`, but see R5. **R5 fails for every member** — token and segment cipher are host-writable, so the on-image locator these projects rely on is exactly the field an adversary can retarget.

**Why this class decides the layout question.** If the scope were confidentiality only, Class L *is* the answer and no new format is needed (§1.3, first row). It falls short only when R5 is added.

---

### 5.2 Class C — stock LUKS2, open contract off the image

**Format.** Byte-for-byte the same as Class L (often with no tokens at all). The profile — which key, which mode, read-only or read-write — lives somewhere else, so the disk alone does not say how to open it.

```
Device       ::=  LUKS2                       -- indistinguishable from Class L
OpenProfile  ::=  <outside the image>         -- guest config | cluster state
                                              -- | host CSI metadata | measured manifest
```

| Member | Where the contract lives | Note |
|--------|--------------------------|------|
| [cryptpilot-crypt](https://github.com/openanolis/cryptpilot/blob/master/cryptpilot-crypt/docs/configuration.md) | `/etc/cryptpilot/volumes/*.toml` in the guest: `dev`, `encrypt.kbs.url`, `encrypt.kbs.resource_path` | LUKS2 `aes-xts-plain64`, `hmac-sha256` when `integrity=true`; most complete KBS story of this class |
| [Contrast](https://github.com/edgelesssys/contrast/security/advisories/GHSA-f5p4-p5q5-jv3h) | cluster secret seed | v1.12.1 ships cryptsetup ≥ 2.8.1; v1.13.0 adds detached header in tmpfs, JSON validation, optional `dm-integrity` (#1734) |
| [Cosmian VM](https://github.com/Cosmian/cosmian_vm/pull/206) | TPM / vTPM binding; file container `/var/lib/cosmian_vm/container` | moving to detached header + `MAC_K(Header)` + `dm-integrity` |
| [dstack](https://github.com/Dstack-TEE/dstack/security/advisories/GHSA-jxq2-hpw3-m5wf) | guest/CVM state for `/data` | now backs up the header to tmpfs, allow-lists the JSON, opens with `--header`; the advisory recommends MACing the whole header |
| [Oasis ROFL](https://github.com/oasisprotocol/oasis-sdk), [Secret Network](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/) | project-specific | same remediation class: allow-list + detached header |
| [CAA CSI](https://confidentialcontainers.org/blog/2026/08/14/encrypted-persistent-storage-for-peer-pods-with-the-caa-csi-block-driver/) | **host** `mountInfo.json`: `encrypt-type`, `kbs-key-id` | the volume never carries `kbs-key-id`; guest detection is `isLuks()` on 6 magic bytes |
| [Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) | Trustee manifest + measured Agent policy | deliberately keeps the profile off the disk; whole-snapshot freshness is an explicit non-goal (§1.2) |

A recurring bug in this class: “if it looks like LUKS, reopen; otherwise format.” Contrast and CDH both had to replace that heuristic, because a host-supplied header turns a failed open into a reformat.

**Properties.** R1 and R4 hold. **R2 and R3 fail**: hand the disk to a CVM that has no guest TOML, no cluster seed, and no Kata policy, and there is nothing to open it with. R5 is being retrofitted through the open procedure rather than the format.

---

### 5.3 Class E — LUKS2 plus an authenticated region on the same device

**Format.** The device stops being “just LUKS2”. A small region, at a fixed offset agreed by the implementation, authenticates the header; the LUKS payload is shifted or detached.

```
Device    ::=  Envelope · Payload

Envelope  ::=  MAC_K( Header )            -- key K not held by the host
             [ · DetachedHeader ]         -- authoritative metadata, off the live path
             [ · CrashState ]             -- generation / journal for fail-closed reopen

Open      ::=  verify Envelope in guest RAM
               cryptsetup open --header H_ram Payload
```

| Member | Layout | Detail |
|--------|--------|--------|
| [tdx-init v0.2.0](https://github.com/flashbots/tdx-init/releases/tag/v0.2.0) | `MACSector_512 · LUKS2` — payload shifted forward one 512-B sector | ToB cites it as the reference MAC-the-header fix. Proves a few hundred bytes of preamble closes the bug. MAC key is an operator secret, not attestation-released |
| [CDH #1648](https://github.com/confidential-containers/guest-components/pull/1648) / [#1663](https://github.com/confidential-containers/guest-components/pull/1663) | `Region(DetachedLUKS2Header, MAC_K(Header), CrashState) · Payload` | zero-only init, fail-closed reopen (never format because open failed), journaled `hmac-sha256` `dm-integrity`. The in-tree CoCo answer |
| [Fortanix Salmiac](https://github.com/fortanix/salmiac) | MAC + detached header | same remediation class, no published layout |

```
+-----------+----------------------------------------------+
| MAC 512 B | LUKS2 (header + keyslots + payload)          |   tdx-init
+-----------+----------------------------------------------+
0           512
```

**Properties.** **R5 is satisfied** — this is the only class that authenticates the contract on its own. R4 holds: the payload is still opened by `cryptsetup`. CDH's crash state additionally makes reopen fail-closed, which is about crash consistency, not freshness. **R2/R3 remain weak**: the region is at a private offset with no type UUID, so a generic agent cannot discover it.

**Why it cannot be adopted as-is.** This class covers `crypt+integrity` completely — RW integrity is in-band, and CDH #1663's journaled `hmac-sha256` is what this repo ends up specifying. It covers **`verity` and `verity+crypt` not at all.** The envelope is by definition a MAC over a *LUKS2 header*, and a LUKS2 header has no field for a verity root hash and no second extent for a hash tree; none of the three members defines one. Adding them means adding discoverable extents and an authenticated place to record their geometry, which is Class M. Full argument in §6.3.

---

### 5.4 Class M — multi-partition images, roles in a partition table

**Format.** More than one on-disk role, made discoverable by a partition table. This is the only class that can host a `dm-verity` hash tree as a first-class object, because a verity tree is a second area with its own size and root hash.

```
Image  ::=  GPT( P_1, P_2, … )
P_i    ::=  LUKS2 | VerityDevice | IntegrityDevice | Filesystem | MetaRegion

Discovery by type UUID + partition label — no plugin, no private offsets.
```

**Member: [systemd-homed](https://systemd.io/HOME_DIRECTORY/)** ([source doc](https://github.com/systemd/systemd/blob/master/docs/HOME_DIRECTORY.md)) — the precedent whose spec is already written in the style we want.

```
HomeImage ::=  GPT( P_home )

P_home.type_uuid  = 773f91ef-66d4-49b5-bd83-d683bf40ad16
P_home.label      = Username

P_home            ::=  LUKS2                       -- label = Username
LUKS2.tokens[i]   ::=  { type:   "systemd-homed",
                         record: Base64( Enc_VK(UserRecordJSON, IV) ),
                         iv:     Base64(IV) }
LUKS2.Payload     ::=  Filesystem ∈ { ext4, btrfs, xfs }
```

A file or removable disk is a complete, self-describing home: identity, how to unlock, how to validate the record. Unlock first, then trust the filesystem. But the metadata is a desktop user record, not attestation + KBS; there is no verity partition; and the header stays host-malleable (Class L caveat).

**Member: [cryptpilot](https://github.com/openanolis/cryptpilot) FDE** — multi-volume, but the anchor is off-disk.

```
FDE_disk ::=  GPT( P_rootfs, P_delta, … )

P_rootfs ::=  [LUKS2] · VerityProtectedRootfs      -- RO root, dm-verity required
P_delta  ::=  LUKS2[ integrity = hmac-sha256? ]    -- MUST be encrypted; overlayfs / COW

Anchor   ::=  metadata.toml ⊂ initrd               -- holds the verity root_hash
              measured with the boot chain, NOT an on-disk authenticated region

Stack:   P_rootfs → [dm-crypt] → dm-verity → … → dm-snapshot → /sysroot
         P_delta  → dm-crypt   → [dm-integrity] → COW
```

Strongest productized “CVM + data disk + KBS” stack ([FDE boot](https://github.com/openanolis/cryptpilot/blob/master/cryptpilot-fde/docs/boot.md), [FDE config](https://github.com/openanolis/cryptpilot/blob/master/cryptpilot-fde/docs/configuration.md), [Alibaba Cloud](https://www.alibabacloud.com/help/en/ecs/user-guide/disk-measurement-and-encryption-protection-for-confidential-computing-instances)). **R3 fails**: the verity root lives in the initrd, so the image cannot describe its own read-only integrity. (The key URI in guest TOML is *not* the failure — measured guest config is a legitimate locator channel, and §1.2 R3 prefers it to an on-disk field.)

**Member: CDF v1 (this repo)** — `GPT(P_meta, [P_verity], P_data)` with the open contract stored in cleartext in `P_meta` under a MAC. See [design/README.md](../design/README.md).

**Properties.** R2/R3 are reachable here and nowhere else, because roles are discoverable without private offsets and there is somewhere to put a verity root. R5 is **not** automatic: homed and cryptpilot are both in this class and both fail it. A partition table buys discoverability, not authenticity — that still requires the Class E mechanism.

---

### 5.5 Class P — platform contracts, not portable images

**Format.** The disk layout is cloud-private, and the contract is an API plus a platform key-release policy. Listed for capability comparison only: none of these can be handed to a different CVM as a self-describing volume, so none can satisfy R2/R3 as a format.

```
Device   ::=  <platform-encrypted block device>   -- layout not published, or not portable
Contract ::=  cloud API + attestation-bound platform key
```

| Platform | What is on the disk | Why it is not a candidate format |
|----------|---------------------|----------------------------------|
| [Azure Confidential VM](https://learn.microsoft.com/en-us/azure/confidential-computing/confidential-vm-overview) | `OSDisk_encrypted_by_platform · VMGS · [guest LUKS \| BitLocker]`. VMGS is a small companion disk holding vTPM / UEFI state; key release is bound to attestation via `SecurityEncryptionType ∈ {DiskWithVMGuestState, …}` | Closest capability match of the whole class — the host cannot read OS-disk plaintext — but the layout is cloud-private and the vTPM anchor lives in a **separate disk**, not in the volume. A guest that stacks its own LUKS inherits every §4.1 gap |
| Google Compute CSEK / CMEK | platform-wrapped DEK; guest sees a normal block device | key contract is the GCP API, not an image header |
| AWS EBS / Nitro Enclaves | EBS: platform AES. Nitro: no attached EBS, secrets via attested KMS. [enclavia-crypto](https://github.com/EnclaviaIO/enclavia) prepends a 4 KiB RSA-OAEP key blob to a LUKS file | the 4 KiB blob is an ad-hoc envelope with no published spec |
| [IBM Secure Execution](https://www.ibm.com/docs/en/linuxonibm/pdf/lx24se04.pdf) | guest data is ordinary LUKS; passphrases are baked into the encrypted SE image / initrd and `/etc/crypttab` | the anchor is the SE header + host-key document, not the data disk |
| [QEMU qcow2 + LUKS](https://github.com/qemu/qemu/blob/master/docs/interop/qcow2.rst) | header extension `0x0537be77` points at a LUKS1 header inside the qcow2 file; payload clusters encrypted, qcow2 metadata cleartext | virtual-image container rather than a raw attachable volume; LUKS1, no `dm-integrity` |

Two of these are still informative: qcow2 and enclavia both use **a small preamble followed by a standard LUKS payload**, which is the same shape as tdx-init's MAC sector and as a GPT meta partition. Azure's VMGS shows the cost of putting the anchor on a second device — it does not travel with the volume.

---

### 5.6 Class X — non-LUKS whole-disk formats, rejected as a base

One line each: the format, and why it is not the base.

| Format | Rejected because |
|--------|------------------|
| [VeraCrypt](https://veracrypt.io/en/VeraCrypt%20Volume%20Format%20Specification.html) — `Salt_64 · Enc_K(Header_448 · MasterKeys) · Reserved · Payload [· HiddenHeader @ 64 KiB]` | Incompatible with `cryptsetup`: no LUKS tokens, no `dm-integrity` stacking. Its plausible-deniability goal (the disk looks like random data) directly contradicts R3, which wants discoverable metadata. |
| [BitLocker / FVE](https://github.com/libyal/libbde/blob/master/documentation/BitLocker%20Drive%20Encryption%20%28BDE%29%20format.asciidoc) — `VolumeHeader("-FVE-FS-") · 3 × FVE_MetadataBlock · EncryptedPayload`, VMK wrapped by protectors, FVEK wrapped by VMK | Incompatible with `cryptsetup`; Windows-only data plane. The idea worth borrowing — on-disk key protectors unsealed by hardware — is already what LUKS2 tokens + TPM2 / KBS do. |
| [TCG Opal via LUKS2](https://gitlab.com/cryptsetup/LUKS2-docs) — `segments[i].type ∈ {hw-opal, hw-opal-crypt}`, `requirement "opal"` | Hardware locking range, not an image format, and the untrusted-host threat model does not trust disk firmware. Keep as an optional segment type, never as the contract. |
| ZFS native encryption, fscrypt, fs-verity, ocicrypt | Wrong layer: they do not produce a block device that CDH can `SecureMount` and a CVM can `cryptsetup open`. |

---

## 6. Comparison

Intent again: `GPT(P_meta_MAC, [P_verity], P_data_LUKS2[+integrity])`; Trustee holds keys; the locator comes from the trusted channel; CDH or a CVM agent opens without Kata.

**LUKS2 header-replace column** (Trail of Bits / CVM host). The host can rewrite the cleartext LUKS2 header. If `open` **fails**, the usual result is availability loss; old AES-XTS ciphertext stays confidential. If `open` **succeeds** with a weakened data plane (or the guest formats after a “successful” open that will not mount), **new writes** can become host-visible. That is not limited to from-scratch disks.

| Cell | Meaning |
|------|---------|
| **yes** | Risk present: attached open trusts the live header; no published MAC / allow-list / measurement. |
| **reduced: detached + allow-list** | ToB option 2. Closes common cipher downgrades and TOCTOU *if* validation and `open` use the same in-RAM copy. Residual: unlisted fields (reencrypt, new tokens). |
| **mitigated: MAC + detached** | ToB preferred. Header bytes are authenticated with a key the host does not hold; `open --header` uses that copy. |
| **n/a** | Not a LUKS2 open contract (different trust root). |

### 6.1 By class

The properties are class properties, so this is the table that matters. The two integrity kinds of §1.1 are separate columns, because that is where the classes actually diverge; the last column is the verdict.

| Class | RW integrity | RO integrity (`dm-verity`) | Open contract on the image? | Portable (R2/R3) | Authenticated contract (R5) | **Why it cannot be used as-is** |
|-------|--------------|----------------------------|-----------------------------|------------------|-----------------------------|---------------------------------|
| **L** stock LUKS2, contract in header | yes — in-band `dm-integrity` | **no** — LUKS2 has no field for a root hash and no second extent for a hash tree | yes — header / `tokens`, **unauthenticated** | partial: needs a plugin or a known token type | **no** — SHA-256 csum ≠ MAC | Two independent failures: the contract is host-writable (R5), and RO integrity cannot be expressed at all (R1) |
| **C** stock LUKS2, contract off-image | yes | **no** — same as L | **no** | **no** — the profile does not travel | retrofitted in the open path (detached + allow-list) | The image does not describe itself (R2/R3), and RO integrity still has nowhere to live (R1) |
| **E** LUKS2 + authenticated region | yes — CDH #1663 journals `hmac-sha256` | **no** — the envelope authenticates *a LUKS2 header*; no root-hash field, no defined hash-tree area | yes, at a **private offset** | weak: no type UUID, not discoverable | **yes** | **RO integrity is unreachable** (R1) and the region is undiscoverable (R2/R3). Both fixes require a role map — which is Class M. See §6.3 |
| **M** multi-partition image | yes | **yes** — a partition can hold the tree, and `P_meta` can hold the root hash | yes, **discoverable**; authenticated only if M + E | yes | only if combined with E | Nothing structural, but the contract is unauthenticated (R5) in every published member, so M alone is still not usable |
| **P** platform contract | platform | platform | n/a — cloud API | no | platform-internal | Not a portable image format at all (R2) |
| **X** non-LUKS formats | varies | varies | yes (own header) | fails R4 | varies | Wrong data plane: not openable by stock `cryptsetup` (R4) |

No class provides device-level freshness; see §1.2.

### 6.2 By project

Ordered as in §5:

| Project | Class | Integrity | Key / policy location | LUKS2 header replace (ToB)? |
|---------|-------|-----------|------------------------|------------------------------|
| LUKS2 (building block) | — | optional integrity | passphrase / token | **yes** — SHA-256 csum ≠ MAC |
| dm-verity (building block) | — | read-only tree | root hash via trusted channel | **n/a** — trust is the root hash, not a LUKS header |
| Clevis / cryptenroll | L | inherits LUKS | Tang / TPM / FIDO2 | **yes** under a CVM host |
| systemd-homed | L + M | LUKS only | user passphrase | **yes** under a CVM host (not its threat model) |
| Constellation | L | optional | cluster KeyService | **reduced: detached + allow-list** (post-advisory) |
| libcryptsetup-token-kbs | L | inherits LUKS | KBS, via token or SMBIOS | **yes** — the plugin fetches the key; the header stays unauthenticated |
| cryptpilot-crypt | C | optional integrity | **guest TOML** + KBS URI | **yes** — open trusts the on-disk header |
| Contrast | C | optional (v1.13) | cluster secret seed | **reduced: detached + allow-list** (v1.13) |
| Cosmian / dstack | C | hardening in progress | TPM / local state | **reduced** — Cosmian: MAC path; dstack: allow-list + detached |
| Oasis / Secret Network | C | JSON check | project-specific | **reduced: detached + allow-list** |
| CAA CSI | C | not specified | KBS id in host `mountInfo.json` | **yes** — `isLuks` is 6 bytes; CDH open trusts the disk header unless #1648 is used |
| Kata #13638 | C | via CDH | measured policy + Trustee manifest | **mitigated if** the CDH envelope is used; the manifest alone does not MAC the header |
| tdx-init | E | header MAC | operator passphrase | **mitigated: MAC + detached** |
| CDH #1648 / #1663 | E | header MAC + optional integrity | KBS | **mitigated: MAC + detached**, plus fail-closed reopen |
| Salmiac | E | MAC | project-specific | **reduced/mitigated** — MAC, no published layout |
| cryptpilot FDE | M | verity + optional integrity | KBS + verity root in **initrd** | **yes** on its LUKS volumes; the verity path is a different bug |
| Azure Conf. OS disk | P | platform | AKV + attestation | **n/a** for the platform disk; **yes** if the guest stacks LUKS2 |
| **CDF v1 (this repo)** | **M + E** | optional verity + integrity | locator from the trusted channel; **no locator on the image** | **mitigated** — MAC over the open contract + detached `open` |

### 6.3 Why the two closest classes still cannot be adopted

Four of the six classes are ruled out quickly: **P** is not a portable image, **X** is the wrong data plane (R4), and **L** / **C** cannot authenticate the open contract (R5) — that much is the Trail of Bits result. The interesting cases are **E** and **M**, because each already solves what the other lacks, and neither is short of a small patch.

**Class E** is the only class that satisfies R5 on its own, so it is the natural candidate for "just use this". It fails on **read-only integrity**:

1. **The envelope is defined as a MAC over a LUKS2 header, and a LUKS2 header cannot describe a verity volume.** RO integrity introduces two objects with no home: the hash tree, which is a second extent with its own size, and the root hash together with salt, hash algorithm, and the two block sizes, which must arrive authenticated from outside the tree it validates. None of tdx-init, CDH #1648/#1663, or Salmiac defines a field for either — they were designed to fix header forgery, not to describe a second protection mode.
2. **`veritysetup --hash-offset` is not a way out.** The tree can technically live at an offset on the same device, but then the format has a private offset for the envelope *and* a private offset for the tree, and still nowhere authenticated to record the root hash. Two undiscoverable conventions instead of one.
3. **Its RW half is fine, and is adopted rather than rejected.** CDH #1663's journaled `hmac-sha256` is precisely what this repo specifies for `crypt+integrity`. The gap is exactly the read-only half of §1.1 — which is why adding RO integrity to the scope, and not anything about encryption, is what disqualifies the class.
4. **The envelope is also undiscoverable.** A fixed offset with no type UUID means a generic agent cannot distinguish an enveloped device from a damaged LUKS2 device, which is the "looks like LUKS, else format" bug in §5.2 with a sharper edge.

Both (1) and (4) are fixed by the same thing: extents with discoverable roles and an authenticated place to record their parameters. That is the definition of Class M. So Class E cannot be *extended* into the answer without becoming M + E.

**Class M** is the mirror image. It has room for the hash tree and somewhere to put the root hash, so it can express every mode in §1.1 — but a partition table is only discovery, not authenticity. Both published members prove it: systemd-homed's threat model has no malicious host, and cryptpilot FDE keeps its verity root in the initrd rather than in an authenticated on-image object, so it fails R3 for exactly the mode under discussion.

**Hence the conclusion.** RO integrity forces M; R5 forces E; nothing surveyed is both. CDF v1 is therefore specified as M + E — a discoverable role map whose metadata partition is authenticated with a key the host does not hold.

---

## 7. Implications for the format we will write down

1. **Do not replace LUKS2.** Every serious Linux CVM/CoCo stack keeps LUKS2 as the ciphertext container so `cryptsetup` remains the data plane (R4). The new specification should be an **envelope + discovery layer** — in the terms of §5, **Class M for discovery plus Class E for authenticity**. No surveyed project combines the two.

2. **“Metadata on disk” is a solved pattern; “authenticated metadata on disk” is not.** Tokens (homed, Clevis, cryptenroll, Constellation, `kbs`) solve discoverability. They do **not** solve a malicious host rewriting the JSON. Any format this repo publishes MUST specify one of:
   - `MAC_K(header)` (tdx-init, Salmiac, CDH #1648), or
   - `AEAD_K(meta)` (an encrypted `P_meta`), or
   - measurement of a sanitized header into attestation before key release,
   and MUST open via **detached header in guest RAM**.

   Between the first two, a **MAC is the better fit** and is what this repo specifies. The threat is a host that rewrites the open parameters, not one that reads them: cipher, integrity profile, verity root, and even the LUKS2 header are public in every other deployment of these primitives. Encrypting them adds nonce management, forces a second cleartext object so the volume can still be recognised before a key exists, and makes the disk unreadable to recovery tooling — for no confidentiality that is actually needed. Both existing Class E implementations reached the same answer. See [design §7.1](../design/README.md#71-why-a-mac-and-not-aead).

3. **Justify `P_meta` against the `kbs` token.** A LUKS token of type `kbs` plus a header MAC already covers “where is the key” for a plugin-equipped CVM. An independent GPT meta partition is worth standardizing only if it also carries, in one authenticated object:
   - the partition map / role (`data`, optional `verity`),
   - the integrity profile (`none | hmac-sha256 | …`),
   - the verity root hash, which has nowhere to live in a LUKS token at all,
   - enough readable metadata to let a generic agent decide “this is a confidential volume” without loading a cryptsetup plugin.

   Note what is deliberately **not** on that list: the key locator. Unlike the `kbs` token, `P_meta` does not name a key service. The locator cannot be authenticated at the time it is needed, and the projects that tried it have moved the same way — the `kbs` token plugin prefers SMBIOS `initdata.toml` over its own on-disk field, and Kata #13638 puts the manifest URI in measured policy. See [design §6.2](../design/README.md#62-key-acquisition-is-out-of-band).

4. **Integrity is two layers.**
   - Immutable / RO: `dm-verity`, root hash in authenticated meta (not only in initrd).
   - Mutable / RW: `dm-integrity` (hmac-sha256). This detects substitution of *blocks*, not substitution of *an older good disk*.

5. **Say plainly that device freshness is not solved.** Host-level snapshot restore always succeeds against integrity tags and against a header MAC, because the adversary rolls back the evidence with the data. The format should state this as a non-goal instead of implying protection through an on-disk counter that the same adversary controls. Deployments that need it add an external anchor above the format.

6. **Reuse published envelopes where they fit.** CDH #1648/#1663 (fail-closed reopen, header MAC, integrity) and tdx-init (MAC preamble) should be treated as implementation proofs of the envelope, not as competing formats. The document this repo will publish is the **stable, Kata-independent layout** those implementations can emit and consume.

7. **Discovery UUIDs.** Follow systemd-homed: mint GPT type UUIDs and LUKS `subsystem` / `label` values so `blkid` / udev can find the image on a raw disk in a CVM that has never heard of CoCo.

---

## 8. References

**Specifications**

- [LUKS2 On-Disk Format Specification](https://gitlab.com/cryptsetup/LUKS2-docs) ([PDF](https://fossies.org/linux/cryptsetup/docs/on-disk-format-luks2.pdf))
- [LUKS1 On-Disk Format](https://gitlab.com/cryptsetup/cryptsetup/-/wikis/LUKS-standard/on-disk-format.pdf)
- [dm-integrity](https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html)
- [dm-verity](https://docs.kernel.org/admin-guide/device-mapper/verity.html) · [cryptsetup DMVerity wiki](https://gitlab.com/cryptsetup/cryptsetup/-/wikis/DMVerity)
- [QEMU qcow2 (LUKS extension)](https://github.com/qemu/qemu/blob/master/docs/interop/qcow2.rst)
- [VeraCrypt Volume Format Specification](https://veracrypt.io/en/VeraCrypt%20Volume%20Format%20Specification.html)
- [BitLocker (BDE) format (libbde)](https://github.com/libyal/libbde/blob/master/documentation/BitLocker%20Drive%20Encryption%20%28BDE%29%20format.asciidoc)
- [systemd Home Directories](https://systemd.io/HOME_DIRECTORY/)
- [TCG Opal / LUKS2 hw-opal](https://gitlab.com/cryptsetup/LUKS2-docs)

**CVM / CoCo implementations**

- [openanolis/cryptpilot](https://github.com/openanolis/cryptpilot)
- [Alibaba Cloud: confidential disk measure & encrypt](https://www.alibabacloud.com/help/en/ecs/user-guide/disk-measurement-and-encryption-protection-for-confidential-computing-instances)
- [Edgeless Constellation encrypted storage](https://docs.edgeless.systems/constellation/architecture/encrypted-storage)
- [Edgeless Contrast GHSA-f5p4-p5q5-jv3h](https://github.com/edgelesssys/contrast/security/advisories/GHSA-f5p4-p5q5-jv3h)
- [Cosmian/cosmian_vm](https://github.com/Cosmian/cosmian_vm)
- [Dstack-TEE/dstack GHSA-jxq2-hpw3-m5wf](https://github.com/Dstack-TEE/dstack/security/advisories/GHSA-jxq2-hpw3-m5wf)
- [flashbots/tdx-init](https://github.com/flashbots/tdx-init)
- [latchset/clevis](https://github.com/latchset/clevis)
- [systemd-cryptenroll](https://www.freedesktop.org/software/systemd/man/latest/systemd-cryptenroll.html)
- [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs)
- [CoCo CAA CSI encrypted persistent storage](https://confidentialcontainers.org/blog/2026/08/14/encrypted-persistent-storage-for-peer-pods-with-the-caa-csi-block-driver/)
- [Kata #13638 confidential persistent block storage](https://github.com/kata-containers/kata-containers/issues/13638)
- [guest-components#1648](https://github.com/confidential-containers/guest-components/pull/1648) · [#1663](https://github.com/confidential-containers/guest-components/pull/1663) · [#1691](https://github.com/confidential-containers/guest-components/pull/1691)
- [Azure Confidential VM](https://learn.microsoft.com/en-us/azure/confidential-computing/confidential-vm-overview)
- [IBM Secure Execution](https://www.ibm.com/docs/en/linuxonibm/pdf/lx24se04.pdf)

**Attacks and remediations**

- [Trail of Bits: LUKS2 in confidential VMs (2025-10-30)](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)
- [HackTricks: LUKS2 header malleability / null cipher](https://cloud.hacktricks.wiki/en/pentesting-cloud/confidential-computing/luks2-header-malleability-null-cipher-abuse.html)

---

## Appendix: local demo

A defensive observation of LUKS2 cleartext headers and detached-header open is under [demos/luks2-detached-header/](demos/luks2-detached-header/README.zh.md).
