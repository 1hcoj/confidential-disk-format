# confidential-disk-format

## Aim

This repo is to define a disk format for CoCo community. The expected attributes of the disk format includes

1. (Optional) Confidentiality and integrity protection. The key to decrypt the volume should be provided out-of-band (Attestation + Trustee is one way).
2. Extensibility. Means not only CoCo could use it, CVM scenarios w/o CoCo still works.
3. Self-containerized. Metadata is transparent to users. The image describes how to interpret itself; it does **not** carry the key or the key locator, both of which come from the deployment.
4. Out-of-box.
5. Authentic open contract. The metadata that decides how the volume is opened must be authenticated with a key the host does not hold. This will help to resolve [the vulnerability proposed by bits of trails](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/), in a way where metadata firstly checked and then used in a detached style.



## Steps

To avoid re-invent the wheel, we firstly do an investigation job upon the ecosystem to find if there are any formats that can have such abilities.

The survey of existing formats is in [investigate/README.md](investigate/README.md) ([中文](investigate/README.zh.md)). There are many different projects.
Different project has different disk format, but we can we can classify them based on their fundamental design principles. 

To achieve the target idea mentioned above, some more concrete engineering goals are given to have a clear picture where we want to head for:

1. `RW integrity` and `RO integrity` support. It's derived from (Optional) Confidentiality and integrity protection. Where `rw` can be achieved by `dm-crypto (with luks2)` + `dm-integrity`, and `ro` can be achieved by `dm-verity` + (optional) `dm-crypt (luks2)`. Each case, we should support integrity protection.
2. `Open contract on the image`. For Extensibility and Self-containerized, A metadata (contract) should be on the image. The metadata should include the type of the disk (`ro/rw`), `size`, HMAC of the LUKS2 header, dm-verity root hash etc.

be less customized, and for the main part it should be re-usable.
3. `Portable`. With the image itself, it's enough to describe how to mount the disk image itself.
4. `Authentic open contract`. It means Authenticated contract. The metadata part should be authentic and thus
integrity protected.

Also, it's better have a clear abstraction layer design. That means
    - A hierarchical structure where the top level indicates whether the current volume is encrypted or unencrypted, and whether it is read-only or read-write.
    - Technologies operating at the layer just below the top level, such as LUKS overlay partitions for encryption and dm-verity overlay partitions for read-only integrity.
    - Avoid coupling top-level metadata to lower-level structures.
    In this way, from the image format itself we have clearer semantics and division of responsibility and harder to misuse. 

Then, conduct an analysis of each
subclass as a whole. The classifications can be found in [the doc](investigate/README.md#5-formats-grouped-by-class).

For each class, we analyze whether it's applicable to our needs in [the table](./investigate/README.md#61-by-class).

No existing standard meets all five goals. 

So, we need to define a new disk format to work. There are two basic ideas

1. Use LUKS container format. Use LUKS header's token (See section 3.6 of [the doc](https://fossies.org/linux/cryptsetup/docs/on-disk-format-luks2.pdf)) to take the metadata. For `rw` disks, it's natural. For `ro` disk, use null cipher. The dm-verity hash tree follows the LUKS header, followed by the data area.
2. Use GPT partition format. Use the first partition to take metadata. Optional second partition to work as dm-verity tree. The third partition as (encrypted) data. See [design/README.md](design/README.md) ([中文](design/README.zh.md)).


| Comparison item                        | Pure LUKS (Way 1)                                                                                                                                                                               | GPT (Way 2)                                                                                                                                                                                                                                                                                           |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Compact / space                        | More. Payload and metadata sit in one LUKS2 object with little padding.                                                                                                                         | Less. Protective MBR + GPT, partition alignment, and a dedicated `P_meta`                                                                                                                                                                                                                             |
| Opener complexity (RW only)            | Smaller. Identify LUKS2, read a token / MAC region, detached `open`.                                                                                                                            | Larger. Parse GPT, select by type UUID, then the same LUKS2 data plane.                                                                                                                                                                                                                               |
| Semantic layering                      | Weaker. The ciphertext container is also the envelope: profile, HMAC, and (for RO) layout live in LUKS2 JSON / tokens.                                                                          | Stronger. GPT + `P_meta` = discovery and authenticated contract; LUKS2 = payload only.                                                                                                                                                                                                                |
| RO / `dm-verity`                       | Awkward. LUKS2 has no first-class second extent or root-hash field. Typical workaround: `cipher_null`, a private hash offset, root hash in a token.                                             | Natural. Optional `P_verity` plus root hash (salt, geometry) in the MAC-covered contract.                                                                                                                                                                                                             |
| Where the authenticated anchor lives   | A mac field with no value should be used to calculate the HMAC of the LUKS header. Then the calculated value is set to the header. The verifier would do a same mechanism to calculate the mac. | `P_meta` is a discoverable partition. HMAC covers the contract and is decoupled from LUKS2 JSON.                                                                                                                                                                                                      |
| Discoverability                        | Weak. Looks like ordinary LUKS.                                                                                                                                                                 | Strong: GPT type UUIDs + labels. A CVM with no CoCo can still find the roles.                                                                                                                                                                                                                         |
| Misuse / misleading                    | High. Tools and humans treat it as stock LUKS. Null-cipher “LUKS” for unencrypted or verity volumes looks like encryption when it is not, which is the Trail of Bits class of confusion.        | Lower. Dedicated type UUIDs mean “this is a CDF image”. `plain` / `verity` are explicit profiles, not a fake LUKS wrapper.                                                                                                                                                                            |
| Non-crypt profiles (`plain`, `verity`) | Forced to keep a LUKS shell or a second ad-hoc path.                                                                                                                                            | Expressible without pretending to be encrypted.                                                                                                                                                                                                                                                       |
| Data plane reuse                       | LUKS2 is the whole image.                                                                                                                                                                       | Data plane is still LUKS2 + `dm-integrity` / `dm-verity`. GPT does not replace cryptsetup.                                                                                                                                                                                                            |
| Future profiles (non-LUKS payload)     | Hard. Everything is interpreted as a LUKS object.                                                                                                                                               | Easier. New `profile` + partition role; contract stays the top-level object.                                                                                                                                                                                                                          |
| Layout migration (RW today, RO later)  | v1 = LUKS disk, v2 = GPT disk for the same `rw`. Opener needs an out-of-band version or a heuristic try-one-then-the-other.                                                                     | One layout from v1. RW vs RO is a contract `profile`, not a second envelope.                                                                                                                                                                                                                          |
| Tooling                                | `cryptsetup` only.                                                                                                                                                                              | `blkid` / `sgdisk` to find partitions, then `cryptsetup` / `veritysetup`. Standard tools, one extra step.                                                                                                                                                                                             |
| How to reuse in external projects      | Must parse LUKS2 JSON and understand a private token (often a cryptsetup plugin).                                                                                                               | Read GPT by type UUID, verify `P_meta`, then open the payload as documented.                                                                                                                                                                                                                          |


LUKS2 can certainly meet the requirements within the CoCo scope. However, the investigation reveals that the industry currently
lacks a design format that simultaneously supports the aforementioned attributes. If we look beyond CoCo and consider a standard
format to serve as a universal secure volume design for potential confidential computing scenarios, the GPT design is clearly more
intuitive. This would also allow full-disk encryption schemes in external projects to adopt this format. Thus it have potential
for broader use cases.

A common question is whether GPT adds attack surface that the LUKS-only envelope does not. The storage adversary already rewrites every byte. Forging GPT is equivalent to pointing the opener at the wrong extents; the open path must refuse until the contract MAC verifies, and must not take device-mapper parameters from the table. That is the same fail-closed rule as MAC + detached LUKS open, applied to discovery rather than to the cipher JSON.

## CoCo integration (user stories)

End-to-end CoCo usage (existing disk vs dynamic PVC, Trustee key IDs, YAML, CSI/CDH/Kata boundaries) is drafted for review in:

- [docs/coco-user-story.zh.md](docs/coco-user-story.zh.md)（中文主文档）
- [docs/coco-user-story.md](docs/coco-user-story.md)
- Example manifests: [docs/examples/coco-csi/](docs/examples/coco-csi/)

Local GPT demo tool: [demo/README.md](demo/README.md).

CSI / guest-components / kata-containers code changes are intentionally **not** started until that user-story doc is approved.

## AI Helped

The Data collection and most word work are helped with AI. But the core idea, the basic logic as following are enforced by human being.

1. Avoid re-inventing the wheels if possible. A data collection upon open-source is needed.
2. Make clear what we need. Basic requirements in google doc and also RO+integrity volumes.
3. Final design. AI helps to do word work.

