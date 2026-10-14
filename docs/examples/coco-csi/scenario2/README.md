# Scenario 2 — bare-metal dynamic PVC, Pod1 then Pod2

Uses the **same** StorageClass as scenario 1: [`../storageclass.yaml`](../storageclass.yaml)
(`cdf-crypt-integrity`, `cdf.platform=bare-metal`).

1. Key Manager creates `K_root` at the URI in the SC/PVC annotation.
2. Apply SC (once) + PVC + pod1 → write; delete pod1 → pod2 → read.

```bash
kubectl apply -f ../storageclass.yaml
kubectl apply -f pvc.yaml -f pod1.yaml
# …
kubectl delete pod cdf-s2-pod1
kubectl apply -f pod2.yaml
```

No pre-created PV: the provisioner runs `CreateVolume` (fetch key by URI, seal file under `cdf.volume-dir`).
