# CoCo CDF 用户故事与接入约定（评审定稿）

**状态：** 评审结论已锁定（见 §0）；可据此实现 CSI / guest-components / kata-containers  
**范围：** GPT 版 CDF v1（[`design/README.zh.md`](../design/README.zh.md)）+ CoCo（裸金属 Kata 与 PeerPods 共用同一条 guest 路径）  
**English：** [`coco-user-story.md`](coco-user-story.md)

本地工具见 [`demo/README.md`](../demo/README.md)。

---

## 0. 已锁定的评审结论

| # | 议题 | 结论 |
|---|------|------|
| 1 | 密钥供给 | **Key Manager** 独立就绪；CSI 消费完整 `kbs://` URI |
| 2 | CDH API | **`volume_type=cdf`** |
| 3 | mkfs | **guest（CDH）**，open 后若无 FS 再 mkfs |
| 4 | 裸金属后端 | **文件 + losetup** |
| 5 | RuntimeClass | 裸金属示例 **`kata`**；PeerPods 示例 **`kata-remote`** |
| 6 | 盘上 locator | 可选 **`Contract.key_locator`**；K8s 覆盖盘上（见下） |
| 7 | `cdf.access-mode` | **删除**（由 `cdf.profile` 推导） |
| 8 | `cdf.expected-volume-uuid` | **保留**，由 **CDH** 在取钥/打开前强制比对（可选） |

### Locator 优先级（部署 > 盘）

```text
PVC 注解 cdf.confidentialcontainers.org/kbs-key-id
  > StorageClass 参数 cdf.kbs-key-id
  > 静态 PV volumeAttributes cdf.kbs-key-id
  > Contract.key_locator（盘上可选 hint）
  > 皆空 → 报错
```

`volume_uuid` 仍随机生成、只在盘上；与 KBS 路径解耦。造盘可写 `--key-locator` 进 Contract，方便无 K8s 的 CVM；CoCo 下以 PVC/SC/PV 为准。

### 平台区分（裸金属 vs PeerPods）

| 属性 | 取值 | 含义 |
|------|------|------|
| **`cdf.platform`** | `bare-metal` \| `peer-pods` | **必填**（SC 或 PV）。决定 NodePublish / 热插路径 |
| `cdf.host-path` | 节点本地文件路径 | 仅 `bare-metal` |
| `cdf.cloud-provider` | `aws` \| `azure` \| `libvirt` | 仅 `peer-pods` |
| `cdf.cloud-volume-id` | 云盘 ID（如 `vol-0abc…` / Azure disk resource ID） | PeerPods **预置**盘时必填 |

参考 PeerPods 块热插：[caa-csi-block-driver](https://github.com/confidential-devhub/caa-csi-block-driver)（`mountInfo.json` → CAA → PodVM）。

---

## 1. 目标场景

| 场景 | 平台 | 意图 | 盘如何就绪 |
|------|------|------|------------|
| **S1** | 裸金属 | 已有本地 CDF 文件 → Pod | 集群外 `cdf-gpt create`；静态 PV + `host-path` + `nodeAffinity` |
| **S2** | 裸金属 | 动态 PVC，Pod1→Pod2 | CSI `CreateVolume`：取钥 seal 本地文件 |
| **S3** | PeerPods | 预置云盘（已有 cloud disk id）→ Pod | 云上已有盘（可先离线 seal 再挂）；静态 PV 带 `cloud-volume-id` |
| **S4** | PeerPods | 动态云盘 PVC | CSI/CAA 建云盘并 seal（或等价供给路径）；guest 只 open |

Guest 侧（四场景相同）一旦块设备进 VM：

```text
mountInfo.json (volume-type=cdf, metadata…)
  → kata-agent → CDH SecureMount(volume_type=cdf)
  → 探测裸盘：空盘 / 已 seal 的 CDF / 其它内容
  → 空盘：rootfs 策略允许该 locator 创建 → 取 K_root → GPT + LUKS2 + mkfs → 最后写元数据
  → CDF：解析 locator（部署 > 盘上 key_locator）→ 取 K_root → HKDF(volume_uuid) → MAC
         → 策略检查 volume_uuid → mapper → mount（open 路径从不 mkfs）
  → 其它内容：拒绝，不覆盖
```

---

## 2. 组件职责

```text
 Key Manager / 用户                CSI (本仓)
  预先写入 K_root ──kbs URI──►  CreateVolume: 只分配空的稀疏文件（host 不持 K_root）
  （CSI 不创建密钥）              NodePublish: losetup + DirectVolume MountInfo
                                        │
                                        ▼
                               kata-runtime / kata-agent
                                        │ SecureMount volume_type=cdf
                                        ▼
                               CDH: 证明后按 URI 取 K_root → 空盘按策略 create / 否则 open → mount
                                        │
                                        ▼
                                     Trustee/KBS
```

| 组件 | 仓库 | 做什么 | 不做什么 |
|------|------|--------|----------|
| **cdf-gpt** | 本仓 `demo/` | 离线 create / info / open | 集群控制面 |
| **cdf 库** | 本仓 `cdf/` | create / probe / inspect / verify / activate，cdf-gpt 与 CDH 共用 | — |
| **CDF CSI** | 本仓 `csi/` | 接受已有 `kbs-key-id`；S2 分配空盘；文件+loop；写 MountInfo | **不**持有 K_root、不写 CDF 元数据；不创建/注册 Trustee 密钥 |
| **kata-agent** | `../kata-containers` | `volume-type=cdf` → CDH | 不解析 CDF、不持钥 |
| **CDH** | `../guest-components` | `volume_type=cdf`：空盘按 `/etc/cdf/policy.json` 创建；已有卷验 MAC + 策略后 open、mount；部署 locator 空则回落 `Contract.key_locator` | open 路径从不 mkfs；MAC 失败或非 CDF 内容不 format |
| **Key Manager** | 部署侧 | 独立生命周期管理 `K_root` 与 URI | — |

与 ephemeral LUKS（`encryption_key=ephemeral`）路径分离，参数不混用。

---

## 3. 密钥模型

```text
盘上：随机 volume_uuid + 可选 key_locator + MAC'd Contract（无 K_root）
部署：可选 kbs-key-id（完整 URI，覆盖盘上）
guest：locator →（证明后）K_root；volume_uuid → HKDF → 验 MAC
```

### 3.1 S1：用户 / Key Manager 登记

```bash
openssl rand -out ./k_root.bin 32
# Trustee：default/cdf/orders-db ← k_root.bin

sudo ./cdf-gpt ./disk-rw.img create \
  --profile crypt+integrity --size 1G --key-file ./k_root.bin \
  --key-locator 'kbs:///default/cdf/orders-db'   # 可选写入盘上 hint

# CoCo 下仍建议在 PV/PVC 写同一 URI（覆盖/钉死部署侧）
```

### 3.2 S2：Key Manager 先行，CSI 只消费 URI

```text
1. Key Manager 创建密钥 → kbs:///default/cdf/my-pvc-key   （已就绪）
2. 用户创建 PVC，注解或 SC 参数带上该 URI
3. CSI CreateVolume：
     - 读取 URI（PVC > SC），只作为非机密 locator 透传
     - 在 volume-dir 下分配全零稀疏文件；不取钥、不 seal、不写 Trustee
4. Pod1：guest 证明后用 URI 取 K_root；盘为空且策略允许 → GPT + LUKS2 + mkfs，最后 seal 元数据 → mount
   （CDH 日志打印新 volume_uuid）
5. Pod2：只 open（验 MAC，不再 create / mkfs）
6. 运维把 volume_uuid 写入策略的 volume_uuids 绑定；此后盘被擦成空盘也会被拒绝而不是重建
```

### 3.3 Guest 策略（MVP：rootfs 内 `/etc/cdf/policy.json`）

```json
{"volumes":[{"key_locator":"kbs:///default/cdf/my-pvc-key","allow_create":true,"volume_uuids":[],"profile":"crypt+integrity"}]}
```

- `allow_create` 且 `volume_uuids` 为空：空盘可被初始化一次（未绑定状态）。
- `volume_uuids` 非空：只允许打开这些卷；空盘一律拒绝。
- 未列出的 locator：可打开在其密钥下 MAC 通过的卷，但不能创建。
- 迁移到其它 CoCo 部署：镜像 + K_root + volume_uuid，在目标策略中登记为已绑定。
- 整盘回滚（用旧镜像替换）不在 MVP 范围内，需外部状态锚（如 KBS 插件）。

---

## 4. 端到端数据面约定

### 4.1 MountInfo（CSI → Kata / CAA）

**裸金属**（device 已是 guest 可见块设备，或 host loop 经 DirectVolume 传入）：

```json
{
  "volume-type": "cdf",
  "device": "/dev/loopN",
  "fstype": "ext4",
  "metadata": {
    "cdf.platform": "bare-metal",
    "cdf.profile": "crypt+integrity",
    "cdf.kbs-key-id": "kbs:///default/cdf/orders-db",
    "cdf.expected-volume-uuid": ""
  },
  "options": ["defaults"]
}
```

**PeerPods**（device 字段为云盘句柄，供 CAA 附加；与 [caa-csi-block-driver](https://github.com/confidential-devhub/caa-csi-block-driver) 一样写 `mountInfo.json`，由 CAA 挂到 PodVM）：

```json
{
  "volume-type": "cdf",
  "device": "vol-0abc123",
  "fstype": "ext4",
  "metadata": {
    "cdf.platform": "peer-pods",
    "cdf.cloud-provider": "aws",
    "cdf.cloud-volume-id": "vol-0abc123",
    "cdf.profile": "crypt+integrity",
    "cdf.kbs-key-id": "kbs:///default/cdf/orders-db",
    "cdf.expected-volume-uuid": ""
  },
  "options": ["defaults"]
}
```

| 字段 | 含义 |
|------|------|
| `cdf.platform` | `bare-metal` / `peer-pods`，**必填** |
| `cdf.profile` | 期望 profile；兼作 rw/ro（`crypt+integrity`/`plain`→rw，`verity*`→ro）。**不再单独传 access-mode** |
| `cdf.kbs-key-id` | 完整 URI；空则 CDH 回落 `Contract.key_locator` |
| `cdf.expected-volume-uuid` | 可选；**由 CDH 强制检查**（非空则必须等于盘上 UUID，否则拒开） |
| `cdf.cloud-*` | 仅 PeerPods |

### 4.2 CDH `volume_type=cdf`

```text
volume_type     = "cdf"
deviceId/Path   = …          # guest 内已出现的块设备
kbsKeyId        = optional   # 部署 locator；空则用 Contract.key_locator
expectedUuid    = optional   # 非空则与 Superblock.volume_uuid 比对，失败即拒
expectedProfile = "crypt+integrity"
filesystemType  = "ext4"
```

```text
1. 探测裸盘（不经 dm-integrity）：
   - 空盘：前 33 MiB 与末 1 MiB 全零，或 CDF 分区存在但 Superblock 全零（create 中断）
   - CDF：CDF 分区 + 非零 Superblock
   - 其它：拒绝，不覆盖
2. 空盘：locator := kbsKeyId；策略须 allow_create 且未绑定 volume_uuids；
   expectedUuid 非空 → 拒绝；profile 取自策略 → 取 K_root → create（mkfs 在 seal 前）
3. 读 volume_uuid；若 expectedUuid 非空且不一致 → 拒绝
4. locator := kbsKeyId 或 Contract.key_locator
5. 取 K_root → HKDF → 验 MAC（失败不 format）
6. 策略：volume_uuids 非空则必须包含该 uuid；profile 须一致
7. 按 Contract.profile 激活（profile 已隐含 rw/ro）
8. mount（ro profile 用 ro；失败不 mkfs）
```

### 4.3 边界

| 步骤 | 谁做 |
|------|------|
| S0→S1 seal | guest CDH（空盘 + 策略允许），或受信机器上的 `cdf-gpt` |
| S1→mapper | CDH open |
| mapper→FS | create 时 CDH mkfs；open 只 mount |
| MAC 失败 / 非 CDF 内容 / 已绑定却为空盘 | 拒绝 |

---

## 5. 场景 1：已有盘

与场景 2 **共用**同一 StorageClass：[`examples/coco-csi/storageclass.yaml`](examples/coco-csi/storageclass.yaml)（`cdf-crypt-integrity`）。  
差别只在「已有静态 PV + `cdf.host-path`」，不必再声明第二个 SC。

```bash
# Key Manager：kbs:///default/cdf/orders-db ← k_root.bin
sudo ./cdf-gpt ./disk-rw.img create \
  --profile crypt+integrity --size 1G --key-file ./k_root.bin
# 编辑 scenario1/pv-pvc.yaml 中的 host-path 与 kbs-key-id
kubectl apply -f examples/coco-csi/storageclass.yaml
kubectl apply -f examples/coco-csi/scenario1/
```

清单：[`examples/coco-csi/scenario1/`](examples/coco-csi/scenario1/)。

---

## 6. 场景 2：动态 PVC

共用同一 SC。无预创建 PV 时走 `CreateVolume`。

1. Key Manager 准备好 URI（例 `kbs:///default/cdf/my-pvc-key`）。  
2. SC 默认 `cdf.kbs-key-id`；PVC 注解可覆盖。  
3. `kubectl apply` SC + PVC + pod1 → 写入；删 pod1 → pod2 读出。  

CreateVolume：**不**生成也**不**读取密钥，只分配空文件。  
create（GPT + LUKS2 + mkfs + seal）：首挂由 guest CDH 按策略完成。

清单：[`examples/coco-csi/scenario2/`](examples/coco-csi/scenario2/)。

---

## 7. 场景 3：PeerPods 预置云盘

对齐 [caa-csi-block-driver](https://github.com/confidential-devhub/caa-csi-block-driver)：CSI 不把盘挂到 worker，而是写 `mountInfo.json`，由 **CAA** 把云盘挂到 **PodVM**。

### 7.1 准备

1. 云上已有一块盘（AWS EBS / Azure Managed Disk / Libvirt raw），且已是 CDF（可在跳板机 `cdf-gpt create` 后做成快照/镜像，或对空盘在受控环境 seal）。  
2. Key Manager 已有对应 `K_root`。  
3. 记下 **cloud volume id**（如 `vol-0abc…` 或 Azure disk ARM id）。

### 7.2 静态 PV

```yaml
# 要点，完整清单见 examples/coco-csi/scenario3/
cdf.platform: peer-pods
cdf.cloud-provider: aws          # 或 azure / libvirt
cdf.cloud-volume-id: "vol-0REPLACE"
cdf.profile: crypt+integrity
cdf.kbs-key-id: "kbs:///default/cdf/orders-db"
# runtimeClassName: kata-remote
```

流程：PVC 绑定 → NodePublish 写 mountInfo（含 cloud-volume-id）→ CAA AttachVolume → PodVM 出现块设备 → CDH open。

清单：[`examples/coco-csi/scenario3/`](examples/coco-csi/scenario3/)。

---

## 8. 场景 4：PeerPods 动态云盘

1. Key Manager 预先放好 `K_root` + URI。  
2. StorageClass：`cdf.platform=peer-pods`，`cdf.cloud-provider=…`，以及 provider 参数（订阅/区域等，可与 caa-csi 同类参数共存或复用其 provider）。  
3. `CreateVolume`：云 API **创建空盘** →（推荐）控制器临时 attach 到供给节点跑 `cdf-gpt create` seal → detach → 返回 PV（带新 `cloud-volume-id`）。  
4. Pod：`kata-remote`；CAA 附加 → CDH 首挂对空盘按策略 create，之后只 open。  
5. Pod2：同一 PVC，仅 open。

若第一期暂不做「临时 attach seal」，可文档化降级：CreateVolume 只建空云盘，guest 在空盘上做 **一次性** CDF initialize（仅当 `cdf.platform=peer-pods` 且 meta 全零且策略允许）——与裸金属「open 永不 format」分支不同，实现时需显式开关。

清单：[`examples/coco-csi/scenario4/`](examples/coco-csi/scenario4/)。

---

## 9. YAML 字段

### StorageClass

裸金属（S1/S2 共用）：[`examples/coco-csi/storageclass.yaml`](examples/coco-csi/storageclass.yaml)

```yaml
parameters:
  cdf.platform: "bare-metal"
  cdf.profile: "crypt+integrity"
  cdf.fstype: "ext4"
  cdf.kbs-key-id: "kbs:///default/cdf/my-pvc-key"
  cdf.volume-dir: "/var/lib/cdf-csi/volumes"
```

PeerPods（S3/S4 共用）：[`examples/coco-csi/storageclass-peerpods.yaml`](examples/coco-csi/storageclass-peerpods.yaml)（`cdf.platform=peer-pods` + cloud provider 参数）。

**不使用 `cdf.access-mode`**：rw/ro 由 `cdf.profile` 推导。

### PVC 注解

```yaml
metadata:
  annotations:
    cdf.confidentialcontainers.org/kbs-key-id: "kbs:///default/cdf/orders-db"
    # 可选：CDH 强制比对盘上 UUID
    cdf.confidentialcontainers.org/expected-volume-uuid: "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
```

### 静态 PV（S1 裸金属）

```yaml
volumeAttributes:
  cdf.platform: "bare-metal"
  cdf.host-path: "/var/lib/cdf-volumes/disk-rw.img"
  cdf.profile: "crypt+integrity"
  cdf.kbs-key-id: "kbs:///default/cdf/orders-db"
```

### 静态 PV（S3 PeerPods）

```yaml
volumeAttributes:
  cdf.platform: "peer-pods"
  cdf.cloud-provider: "aws"
  cdf.cloud-volume-id: "vol-0REPLACE"
  cdf.profile: "crypt+integrity"
  cdf.kbs-key-id: "kbs:///default/cdf/orders-db"
```

### Pod RuntimeClass

| 平台 | runtimeClassName |
|------|------------------|
| 裸金属 | `kata` |
| PeerPods | `kata-remote` |

---

## 10. 实现分期

| 阶段 | 内容 | 仓库 |
|------|------|------|
| P1 | CDH `volume_type=cdf`（空盘按策略 create、open、key_locator 回落、expectedUuid） | guest-components ← upstream/main |
| P2 | kata-agent `volume-type=cdf` → SecureMount | kata-containers ← upstream/main |
| P3 | 本仓 CSI：S1/S2（`cdf.platform=bare-metal`） | 本仓 `csi/` |
| P4 | PeerPods：S3 预置 + S4 动态（复用/对接 [caa-csi-block-driver](https://github.com/confidential-devhub/caa-csi-block-driver) 热插） | 本仓 CSI + CAA 集成 |

---

## 11. FAQ（容量 / locator / host-path / 属性）

### 11.1 场景 1：`PV.spec.capacity` 和真实盘大小

`capacity` 是给 **Kubernetes 调度 / PVC 匹配** 用的声明值，不会自动从文件读出。

约定：

| 角色 | 行为 |
|------|------|
| 运维写 PV | `capacity` **≤** 真实文件/块设备字节数（通常写成与造盘 `--size` 一致，如 `1Gi`） |
| CSI `NodePublish` / `NodeStage` | `stat`/`blockdev` 量一下 `cdf.host-path`；若 **实际大小 < PVC/PV capacity** → **失败**并打明确错误 |
| CSI 动态 `CreateVolume` | 按 PVC `requests.storage` 创建文件，再写回 PV capacity |

不做「自动改 PV capacity」：静态卷以 YAML 为准，校验只做防护。可用 `cdf-gpt info` 看 `capacity_bytes` 再填 PV。

### 11.2 盘上 `key_locator` 与 K8s 覆盖

规范已增加可选、非秘密字段 **`Contract.key_locator`**（典型为完整 `kbs://…` URI），被 MAC 覆盖。它是**提示**，不是唯一权威来源。

**解析顺序（MUST）：**

```text
PVC 注解 cdf.confidentialcontainers.org/kbs-key-id
  > StorageClass 参数 cdf.kbs-key-id
  > 静态 PV volumeAttributes cdf.kbs-key-id
  > Contract.key_locator（盘上，仅当上面皆空）
  > 皆空 → 报错
```

含义：

- K8s 层写了 key id → **覆盖**盘上字段（换集群 / 换 KBS 路径不必改盘）。  
- K8s 没写 → 可用盘上 hint（方便独立 CVM / 少配 YAML）。  
- 取钥发生在 MAC 之前，盘上/CSI URI 在校验前都不可信；拿错钥 → MAC 失败 → 拒开。  
- `key_locator` **不是**秘密；`volume_uuid` 仍是 HKDF/MAC 身份，与 URI 解耦。

### 11.2.1 `cdf.access-mode`？

**不用。** 与 `cdf.profile` 信息重叠（`crypt+integrity`/`plain`→rw，`verity`/`verity+crypt`→ro）。CDH/mount 的 ro/rw 一律从校验后的 Contract.profile（及部署 `expectedProfile`）推导。

### 11.2.2 `cdf.expected-volume-uuid`？

**是 CDH 层的可选强制检查**：非空时，在取钥或 open 前比对 Superblock.volume_uuid，不一致直接拒绝。CSI 只做透传（注解/PV → MountInfo → SecureMount），不在 host 上打开 CDF。

### 11.3 `host-path` 在哪个 node？

文件只在某台机器上时，**必须**用 PV `nodeAffinity`（或等价 local volume 语义）钉节点；单靠路径字符串调度器不知道文件在哪。

```yaml
spec:
  nodeAffinity:
    required:
      nodeSelectorTerms:
        - matchExpressions:
            - key: kubernetes.io/hostname
              operator: In
              values: ["node-a"]
  csi:
    volumeAttributes:
      cdf.host-path: "/var/lib/cdf-volumes/disk-rw.img"
```

再配合 SC 的 `volumeBindingMode: WaitForFirstConsumer`，PVC 会等到能落到该 node 的 Pod 再绑。

CSI 侧：`NodePublish` 若本机不存在 `host-path` → 失败（路径错误或调度到了错 node）。动态供给则在 **CreateVolume 所在 node** 建文件，并把亲和写进 PV（或依赖 late-binding + 节点上的 volume-dir）。

---

## 12. 相关链接

- 格式规范：[design/README.zh.md](../design/README.zh.md)  
- 本地工具：[demo/README.md](../demo/README.md)  
- 示例 YAML：[examples/coco-csi/](examples/coco-csi/)  
- PeerPods 块驱动参考：[caa-csi-block-driver](https://github.com/confidential-devhub/caa-csi-block-driver)
