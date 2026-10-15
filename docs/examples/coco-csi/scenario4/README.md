# Scenario 4 — PeerPods dynamic cloud disk

Dynamic PVC: controller creates a cloud volume (same provider model as
[caa-csi-block-driver](https://github.com/confidential-devhub/caa-csi-block-driver)),
seals CDF (preferred: temporary attach + `cdf-gpt create`), then CAA attaches
to PodVM. Guest path is identical to S1–S3 (`volume_type=cdf`).

Uses the **same** PeerPods StorageClass as scenario 3:
[`../storageclass-peerpods.yaml`](../storageclass-peerpods.yaml)
(`cdf.platform=peer-pods`).

1. Key Manager pre-creates `K_root` at the URI in SC / PVC annotation.
2. Edit `../storageclass-peerpods.yaml` (region / AZ / Azure subscription, etc.).
3. Apply SC + PVC + pod1 → write; delete pod1 → pod2 → read.

```bash
kubectl apply -f ../storageclass-peerpods.yaml
kubectl apply -f pvc.yaml -f pod1.yaml
# …
kubectl delete pod cdf-s4-pod1
kubectl apply -f pod2.yaml
```

`CreateVolume` must return a PV with `cdf.cloud-volume-id` set and a blank
disk. On first mount CDH creates the CDF volume if the guest policy allows it
for the locator; later mounts only open.
