# CoCo CDF user stories (review locked)

**Status:** Decisions locked — full text in Chinese: [`coco-user-story.zh.md`](coco-user-story.zh.md)

## Locked decisions

1. **Key Manager** owns key lifecycle; CSI only consumes a full `kbs://` URI.  
2. CDH first version: **`volume_type=cdf`**.  
3. **mkfs in guest** after successful open if FS missing.  
4. Bare-metal backend: **file + loop**; PeerPods: cloud disk + CAA attach.  
5. RuntimeClass: bare-metal **`kata`**, PeerPods **`kata-remote`**.  
6. Optional on-disk **`Contract.key_locator`**; deployment overrides disk.  
7. **No `cdf.access-mode`** — rw/ro from `cdf.profile`.  
8. **`cdf.expected-volume-uuid`** — optional **CDH** hard check before open.

### Locator priority

```text
PVC annotation > StorageClass > static PV attributes > Contract.key_locator > error
```

### Platform attribute (bare-metal vs PeerPods)

| Attribute | Values / use |
|-----------|----------------|
| **`cdf.platform`** | `bare-metal` \| `peer-pods` (**required**) |
| `cdf.host-path` | bare-metal only |
| `cdf.cloud-provider` / `cdf.cloud-volume-id` | PeerPods (`cloud-volume-id` required for pre-provision) |

Hot-plug model follows [caa-csi-block-driver](https://github.com/confidential-devhub/caa-csi-block-driver) (`mountInfo.json` → CAA → PodVM).

## Scenarios

| ID | Platform | Flow |
|----|----------|------|
| **S1** | bare-metal | Offline `cdf-gpt create` → static PV + `host-path` + `nodeAffinity` |
| **S2** | bare-metal | Dynamic PVC; CSI `CreateVolume` seals local file |
| **S3** | PeerPods | Pre-provision: static PV with existing **`cdf.cloud-volume-id`** |
| **S4** | PeerPods | Dynamic: create cloud disk (+ seal) → CAA attach → CDH open |

Shared guest path once the block device is in the VM: MountInfo `volume-type=cdf` → kata-agent → CDH SecureMount → locator → `K_root` → MAC → mapper → mkfs? → mount.

Examples:

- Bare-metal SC: [`examples/coco-csi/storageclass.yaml`](examples/coco-csi/storageclass.yaml)  
- PeerPods SC: [`examples/coco-csi/storageclass-peerpods.yaml`](examples/coco-csi/storageclass-peerpods.yaml)  
- S1–S4: [`examples/coco-csi/scenario1/`](examples/coco-csi/scenario1/) … [`scenario4/`](examples/coco-csi/scenario4/)

## Config FAQ (short)

1. **`cdf.access-mode`?** Unused — overlaps `cdf.profile` (`crypt+integrity`/`plain`→rw, `verity*`→ro).  
2. **`cdf.expected-volume-uuid`?** Optional CDH enforcement against `Superblock.volume_uuid`; CSI only passes it through.  
3. **PV capacity:** declarative; bare-metal NodePublish fails if host-path is smaller.  
4. **`key_locator` on disk:** hint only; CoCo prefers PVC/SC/PV URI.

Details: Chinese doc §7–§11.
