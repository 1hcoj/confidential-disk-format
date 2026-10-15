# Bare-metal CoCo: build, replace guest bits, deploy CSI, run S1/S2

This guide assumes you already have a **CoCo bare-metal** cluster (Kata + CDH in the guest image, Trustee/KBS reachable). Workstations here build binaries; you copy them onto the cluster nodes / into the guest image.

## Revisions (local branches)

| Repo | Branch | Tip |
|------|--------|-----|
| `guest-components` | `feat/cdf-volume-type` | `21508a703103dd77e5a8f9ebc62fb719b75a1de3` |
| `kata-containers` | `feat/cdf-volume-type` | `18d7693afd12b8b210e5d4b12e1df923ae027bb9` |
| `confidential-disk-format` | `main` | `6cbef98e64e400e39ab23b3eadd76cbb9eb1727d` (CSI + this guide) |

---

## 1. Guest image: what to replace

You need **two** guest binaries (same paths CoCo already uses):

| Binary | Role |
|--------|------|
| `confidential-data-hub` | Open CDF (`SecureMount` `volume_type=cdf`) |
| `kata-agent` | Detect `volume_type=cdf` → call CDH |

Host-side (worker node, not inside guest image):

| Binary | Role |
|--------|------|
| `containerd-shim-kata-v2` / kata runtime (Go) | Read `mountInfo.json`, hot-plug disk, pass `driver_options` |
| Optional: runtime-rs | Same if your cluster uses it |

### 1.1 Build CDH

```bash
cd /path/to/guest-components
git checkout feat/cdf-volume-type

# CoCo guest binary (musl static on x86_64)
unset CARGO_TARGET_DIR   # avoid sandbox redirects if any
cd confidential-data-hub
make build LIBC=musl ENABLE_KBS=true RPC=ttrpc

# Output:
#   ../target/x86_64-unknown-linux-musl/release/confidential-data-hub
```

Copy into the guest rootfs (typical CoCo layout):

```text
/usr/local/bin/confidential-data-hub
```

Guest still needs `cryptsetup`, `sgdisk`/`gdisk`, `blkid`, `mkfs.ext4` on PATH (already present in coco-extension images).

### 1.2 Build kata-agent

```bash
cd /path/to/kata-containers
git checkout feat/cdf-volume-type
cd src/agent
make

# Output (default):
#   ./target/<triple>/release/kata-agent
# or whatever your packaging uses; see tools/packaging/static-build/agent/
```

Install into guest rootfs where the current agent lives (often):

```text
/usr/bin/kata-agent
```

Rebuild / re-sign the confidential guest image the way you already do for CoCo (`rootfs-image-confidential` / osbuilder / your operator pipeline), then roll nodes or update the RuntimeClass image reference.

### 1.3 Build host kata runtime (Go shim)

```bash
cd /path/to/kata-containers
git checkout feat/cdf-volume-type
cd src/runtime
make

# Install containerd-shim-kata-v2 + configuration on each worker
# (same procedure you use today for Kata upgrades)
```

Without this host update, `mountInfo.json` with `"volume-type":"cdf"` will hot-plug the disk but **won’t** pass `volume_type=cdf` / `cdf.*` to the agent.

---

## 2. CSI: build and deploy

On a machine with Go ≥ 1.22:

```bash
cd /path/to/confidential-disk-format
cd demo && cargo build --release && cd ..
# cdf-gpt used by CreateVolume on the node:
sudo install -m0755 demo/target/release/cdf-gpt /usr/local/bin/cdf-gpt

cd csi
make build
# → csi/bin/cdf-csi
```

Build/load an image for the cluster (example):

```bash
cd csi
make image TAG=dev          # Dockerfile → localhost/cdf-csi:dev
# push or ctr/nerdctl import onto each worker; edit deploy/daemonset.yaml image
```

On **each worker** (or bake into the DaemonSet hostPath mounts):

```bash
sudo mkdir -p /var/lib/cdf-csi/volumes
```

No key material goes on the host. `CreateVolume` only allocates a blank
sparse file; the guest CDH initializes it on first mount.

Deploy:

```bash
kubectl apply -f csi/deploy/namespace.yaml
kubectl apply -f csi/deploy/rbac.yaml
kubectl apply -f csi/deploy/csidriver.yaml
kubectl apply -f csi/deploy/daemonset.yaml
kubectl apply -f docs/examples/coco-csi/storageclass.yaml

kubectl -n cdf-csi-system get pods -o wide
```

Trustee/KBS: register `K_root` at the URI used in SC/PVC annotations so the
**guest** can fetch it after attestation (`cc_kbc`).

Guest rootfs: add `/etc/cdf/policy.json` (see [`csi/README.md`](../csi/README.md#guest-policy)).
A locator must have `"allow_create": true` and no `volume_uuids` before a
blank disk is initialized for it. After first mount, take the `volume_uuid`
from the CDH log (or `cdf-gpt <img> info`, no key needed) and list it under
`volume_uuids` to bind the volume; from then on a wiped disk is refused
rather than re-created.

---

## 3. E2E: Scenario 1 (pre-sealed disk)

The disk may also be a blank file (`truncate -s 1G disk-rw.img`) if the
policy allows creating for its locator; the guest then seals it on first
mount. To pre-seal it instead, do so on a trusted machine (not the worker):

1. Create key and seal offline (on any trusted machine with `cdf-gpt`):

```bash
openssl rand -out k_root.bin 32
# Register k_root.bin in Trustee at kbs:///default/cdf/orders-db

sudo cdf-gpt ./disk-rw.img create \
  --profile crypt+integrity --size 1G --key-file ./k_root.bin \
  --key-locator 'kbs:///default/cdf/orders-db'

sudo cdf-gpt ./disk-rw.img info --key-file ./k_root.bin
```

2. Copy `disk-rw.img` to **one** worker, e.g. `/var/lib/cdf-volumes/disk-rw.img`.

3. Edit `docs/examples/coco-csi/scenario1/pv-pvc.yaml`:
   - `REPLACE_HOST_PATH`
   - `REPLACE_NODE_NAME`
   - confirm `cdf.kbs-key-id`

4. Apply:

```bash
kubectl apply -f docs/examples/coco-csi/storageclass.yaml
kubectl apply -f docs/examples/coco-csi/scenario1/
kubectl exec -it cdf-s1-writer -- sh -c 'echo hello-s1 > /data/hello && cat /data/hello'
```

Expect: Pod `runtimeClassName: kata` → DirectVolume → agent CDH `volume_type=cdf` → mount at `/data`.

---

## 4. E2E: Scenario 2 (dynamic PVC)

1. Register `K_root` at `kbs:///default/cdf/my-pvc-key` in Trustee.

2. Allow creation for that locator in the guest policy (see §2).

3. Apply:

```bash
kubectl apply -f docs/examples/coco-csi/storageclass.yaml
kubectl apply -f docs/examples/coco-csi/scenario2/pvc.yaml
kubectl apply -f docs/examples/coco-csi/scenario2/pod1.yaml
kubectl exec -it cdf-s2-pod1 -- sh -c 'echo hello-s2 > /data/hello && sync'
kubectl delete pod cdf-s2-pod1
kubectl apply -f docs/examples/coco-csi/scenario2/pod2.yaml
kubectl exec -it cdf-s2-pod2 -- cat /data/hello   # expect hello-s2
```

`CreateVolume` allocates a blank file under `/var/lib/cdf-csi/volumes`; pod1's
CDH creates and seals the volume; pod2's CDH only opens it.

---

## Debug tips

| Symptom | Check |
|---------|--------|
| Agent mounts plain FS / fails open | Host runtime not updated; `mountInfo` missing `volume-type: cdf` |
| CDH “unknown volume type” | Guest still on old `confidential-data-hub` |
| MAC verification failed | Guest KBS URI ≠ seal key, or wrong key bytes |
| CreateVolume fails | StorageClass missing `cdf.kbs-key-id` |
| “policy does not allow creating” | Blank disk under a locator without `allow_create`, or bound via `volume_uuids` (wiped volume) |
| “neither blank nor CDF” | Disk has foreign content; CDH never overwrites it |
| Loop device / NodePublish | CSI DaemonSet needs privileged + `/dev` + direct-volumes hostPath |
