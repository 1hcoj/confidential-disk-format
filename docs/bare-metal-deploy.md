# Bare-metal CoCo: build, replace guest bits, deploy CSI, run S1/S2

This guide assumes you already have a **CoCo bare-metal** cluster (Kata + CDH in the guest image, Trustee/KBS reachable). Workstations here build binaries; you copy them onto the cluster nodes / into the guest image.

## Revisions (local branches)

| Repo | Branch | Tip (at doc write time; re-check with `git rev-parse HEAD`) |
|------|--------|--------------------------------------------------------------|
| `guest-components` | `feat/cdf-volume-type` | CDH `volume_type=cdf` |
| `kata-containers` | `feat/cdf-volume-type` | runtime + agent forward `volume-type=cdf` |
| `confidential-disk-format` | `main` (+ CSI commit) | CSI + user stories + `cdf-gpt` |

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
sudo mkdir -p /var/lib/cdf-csi/volumes /var/lib/cdf-csi/provision-keys
# For S2 CreateVolume: place raw 32-byte K_root (or passphrase) where the URI maps:
#   kbs:///default/cdf/my-pvc-key
#   → /var/lib/cdf-csi/provision-keys/default/cdf/my-pvc-key
sudo mkdir -p /var/lib/cdf-csi/provision-keys/default/cdf
sudo cp ./k_root.bin /var/lib/cdf-csi/provision-keys/default/cdf/my-pvc-key
```

Deploy:

```bash
kubectl apply -f csi/deploy/namespace.yaml
kubectl apply -f csi/deploy/rbac.yaml
kubectl apply -f csi/deploy/csidriver.yaml
# Ensure daemonset image + CDF_GPT_BIN=/usr/local/bin/cdf-gpt
kubectl apply -f csi/deploy/daemonset.yaml
kubectl apply -f docs/examples/coco-csi/storageclass.yaml

kubectl -n cdf-csi-system get pods -o wide
```

Trustee/KBS: register the **same** `K_root` bytes at the URI used in SC/PVC annotations so the **guest** can fetch after attestation (`cc_kbc`). Provisioning keys on the host are only for CSI `CreateVolume` seal.

---

## 3. E2E: Scenario 1 (pre-sealed disk)

1. Create key and seal offline (on any machine with `cdf-gpt`):

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

2. Place the same key file for CSI provisioner on workers (see §2).

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

`CreateVolume` seals under `/var/lib/cdf-csi/volumes`; guest only opens (mkfs-if-empty on first mount if needed).

---

## Debug tips

| Symptom | Check |
|---------|--------|
| Agent mounts plain FS / fails open | Host runtime not updated; `mountInfo` missing `volume-type: cdf` |
| CDH “unknown volume type” | Guest still on old `confidential-data-hub` |
| MAC verification failed | Guest KBS URI ≠ seal key, or wrong key bytes |
| CreateVolume seal fails | `cdf-gpt` missing on node; provision key path mismatch |
| Loop device / NodePublish | CSI DaemonSet needs privileged + `/dev` + direct-volumes hostPath |
