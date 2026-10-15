# CDF CSI driver (bare-metal MVP)

Kubernetes CSI plugin for **Confidential Disk Format (CDF)** volumes on bare-metal CoCo nodes (file + loop + Kata DirectVolume). Supports:

- **S1 (static):** an existing image via PV `volumeAttributes.cdf.host-path` (no `CreateVolume`). The image is either a sealed CDF volume or blank.
- **S2 (dynamic):** `CreateVolume` allocates a blank sparse file under `cdf.volume-dir`.

The host never holds `K_root` and never writes CDF metadata. On first mount
the guest CDH finds a blank disk, checks its rootfs policy
(`/etc/cdf/policy.json`), and only then lays out GPT, formats LUKS2 and ext4,
and seals the metadata. Later mounts only open; a sealed volume is never
reformatted, and a blank disk under a locator bound to existing volume UUIDs
is refused. See [`../cdf/`](../cdf/) for the shared create/open library.

Provisioner name: `cdf.csi.confidentialcontainers.org` (matches [`docs/examples/coco-csi/storageclass.yaml`](../docs/examples/coco-csi/storageclass.yaml)).

PeerPods / cloud backends are **not** implemented in this MVP.

## Prerequisites

- Kata Containers runtime with DirectVolume / CDH CDF support (out of tree for this repo).
- A guest CDH policy that allows creating volumes for the StorageClass key locator (see below).

### Guest policy

```json
{
  "volumes": [
    {
      "key_locator": "kbs:///default/cdf/my-pvc-key",
      "allow_create": true,
      "volume_uuids": [],
      "profile": "crypt+integrity"
    }
  ]
}
```

- `allow_create` with empty `volume_uuids`: a blank disk is initialized (CDH logs the new `volume_uuid`).
- non-empty `volume_uuids`: only those volumes open, and a blank disk is refused. Binding is how a volume is protected against a host wiping it, and how it is imported into another CoCo deployment (copy image + `K_root`, list its UUID).
- Locators not listed can open volumes that authenticate under their key, but never create.

### Environment

| Variable | Default | Purpose |
|----------|---------|---------|
| `CDF_VOLUME_DIR` | `/var/lib/cdf-csi/volumes` | Override default volume directory |

## Build

```bash
cd csi
make build          # → bin/cdf-csi
make test
make image TAG=dev  # optional container image
```

Cross-compile example:

```bash
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 make build VERSION=v0.1.0
```

## Deploy

1. Build/push the CSI image and update `deploy/daemonset.yaml` if not using `localhost/cdf-csi:dev`.
2. Apply manifests:

```bash
kubectl apply -f deploy/namespace.yaml
kubectl apply -f deploy/rbac.yaml
kubectl apply -f deploy/csidriver.yaml
kubectl apply -f deploy/daemonset.yaml
kubectl apply -f ../docs/examples/coco-csi/storageclass.yaml
```

### Scenarios

- **S1:** [`docs/examples/coco-csi/scenario1/`](../docs/examples/coco-csi/scenario1/) — set `cdf.host-path` and node affinity.
- **S2:** [`docs/examples/coco-csi/scenario2/`](../docs/examples/coco-csi/scenario2/) — PVC only; provisioner uses node-deployment + `WaitForFirstConsumer`.

For S2 the StorageClass must carry `cdf.kbs-key-id` (`CreateVolume` refuses without a locator). PVC annotation `cdf.confidentialcontainers.org/kbs-key-id` is honored when passed through as a create parameter; with stock external-provisioner, set the URI on the StorageClass.

## Node publish flow

1. `NodeStageVolume`: `losetup --find --show --partscan` on `cdf.host-path`.
2. `NodePublishVolume`: write Kata DirectVolume `mountInfo.json` under  
   `/run/kata-containers/shared/direct-volumes/<base64url(targetPath)>/mountInfo.json`  
   with `"volume-type": "cdf"` and metadata (`cdf.platform`, `cdf.profile`, `cdf.kbs-key-id`, optional `cdf.expected-volume-uuid`).

No filesystem is mounted on the host; the guest creates or opens CDF via CDH.

## Layout

```
csi/
  cmd/cdf-csi/          # single binary (Identity + Controller + Node)
  pkg/driver/           # CSI service implementation
  pkg/cdf/              # blank image allocation + losetup helpers
  pkg/directvolume/     # mountInfo.json writer
  deploy/               # namespace, RBAC, CSIDriver, DaemonSet
  Makefile
```
