# CDF CSI driver (bare-metal MVP)

Kubernetes CSI plugin for **Confidential Disk Format (CDF)** volumes on bare-metal CoCo nodes (file + loop + Kata DirectVolume). Supports:

- **S1 (static):** pre-sealed image via PV `volumeAttributes.cdf.host-path` (no `CreateVolume`).
- **S2 (dynamic):** `CreateVolume` creates a sparse file under `cdf.volume-dir` and seals it with `cdf-gpt`.

Provisioner name: `cdf.csi.confidentialcontainers.org` (matches [`docs/examples/coco-csi/storageclass.yaml`](../docs/examples/coco-csi/storageclass.yaml)).

PeerPods / cloud backends are **not** implemented in this MVP.

## Prerequisites

- Kata Containers runtime with DirectVolume / CDH CDF support (out of tree for this repo).
- `cdf-gpt` binary on each node (build from [`demo/`](../demo/)).
- For dynamic provisioning: host-side key material for sealing (not passed to the guest).

### Provision keys (S2)

`CreateVolume` does **not** call KBS at runtime in this MVP. It reads `K_root` bytes from a file:

| Source | Resolution |
|--------|------------|
| StorageClass / PV param `cdf.provision-key-file` | Use path as-is |
| `cdf.kbs-key-id` or PVC annotation `cdf.confidentialcontainers.org/kbs-key-id` | Map URI → `$CDF_PROVISION_KEY_DIR/<path>` |

Example: `kbs:///default/cdf/my-pvc-key` → `/var/lib/cdf-csi/provision-keys/default/cdf/my-pvc-key`.

Set `CDF_PROVISION_KEY_DIR` in the DaemonSet (default mount in `deploy/daemonset.yaml`).

### Environment

| Variable | Default | Purpose |
|----------|---------|---------|
| `CDF_GPT_BIN` | `cdf-gpt` | Path to seal tool |
| `CDF_PROVISION_KEY_DIR` | (required for URI mapping) | Key file root |
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

1. Build and install `cdf-gpt` on each node (e.g. `/usr/local/bin/cdf-gpt`).
2. Place provision key files under `/var/lib/cdf-csi/provision-keys/…` on nodes that run dynamic provisioning.
3. Build/push the CSI image and update `deploy/daemonset.yaml` if not using `localhost/cdf-csi:dev`.
4. Apply manifests:

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

For S2, ensure the StorageClass `cdf.kbs-key-id` (or matching key file) is present. PVC annotation `cdf.confidentialcontainers.org/kbs-key-id` is honored when passed through as a create parameter (same key name in volume context); with stock external-provisioner, duplicate the URI on the StorageClass or use `cdf.provision-key-file`.

## Node publish flow

1. `NodeStageVolume`: `losetup --find --show --partscan` on `cdf.host-path`.
2. `NodePublishVolume`: write Kata DirectVolume `mountInfo.json` under  
   `/run/kata-containers/shared/direct-volumes/<base64url(targetPath)>/mountInfo.json`  
   with `"volume-type": "cdf"` and metadata (`cdf.platform`, `cdf.profile`, `cdf.kbs-key-id`, optional `cdf.expected-volume-uuid`, `cdf.mkfs-if-empty`).

No filesystem is mounted on the host; the guest opens CDF via CDH.

## Layout

```
csi/
  cmd/cdf-csi/          # single binary (Identity + Controller + Node)
  pkg/driver/           # CSI service implementation
  pkg/cdf/              # cdf-gpt + key path helpers
  pkg/directvolume/     # mountInfo.json writer
  deploy/               # namespace, RBAC, CSIDriver, DaemonSet
  Makefile
```
