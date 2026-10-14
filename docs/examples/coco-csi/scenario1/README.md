# Scenario 1 — bare-metal existing CDF disk (static PV)

Uses the **same** StorageClass as scenario 2: [`../storageclass.yaml`](../storageclass.yaml)
(`cdf.platform=bare-metal`).

1. Put `K_root` in Key Manager at a full URI (e.g. `kbs:///default/cdf/orders-db`).
2. `cdf-gpt create` with that key (optional `--key-locator` on-disk hint); note size via `info`.
3. Copy the image onto **one** node; set in `pv-pvc.yaml`:
   - `REPLACE_HOST_PATH`
   - `REPLACE_NODE_NAME` (`nodeAffinity`)
   - `cdf.kbs-key-id` (deployment locator; overrides disk `key_locator`)
4. Apply SC once, then PV/PVC/Pod (`runtimeClassName: kata`).

```bash
kubectl apply -f ../storageclass.yaml
kubectl apply -f pv-pvc.yaml -f pod.yaml
```

CSI rejects NodePublish if the file is smaller than declared capacity or missing on that node.
No `cdf.access-mode`: rw/ro comes from `cdf.profile`. Optional `cdf.expected-volume-uuid` is enforced by **CDH**.
