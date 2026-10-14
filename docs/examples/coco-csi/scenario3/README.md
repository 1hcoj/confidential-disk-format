# Scenario 3 — PeerPods pre-provisioned cloud disk

Static PV that references an **existing cloud volume id**. Aligns with
[caa-csi-block-driver](https://github.com/confidential-devhub/caa-csi-block-driver):
CSI writes `mountInfo.json`; **CAA** attaches the disk to the **PodVM**.

`cdf.platform=peer-pods` distinguishes this from bare-metal S1 (`host-path` + loop).

1. Create / import a CDF disk in the cloud (offline `cdf-gpt create` → snapshot /
   image, or seal in a controlled environment). Note the cloud id (`vol-0…` /
   Azure disk ARM id / libvirt path).
2. Register `K_root` at a full URI in Key Manager.
3. Edit `pv-pvc.yaml`: `REPLACE_CLOUD_VOLUME_ID`, provider, `kbs-key-id`.
4. Apply PeerPods SC once, then PV/PVC/Pod (`runtimeClassName: kata-remote`).

```bash
kubectl apply -f ../storageclass-peerpods.yaml
kubectl apply -f pv-pvc.yaml -f pod.yaml
```

No `cdf.host-path`. Optional `cdf.expected-volume-uuid` is enforced by **CDH** after attach.
