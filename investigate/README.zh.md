# 调研：带盘上元数据的机密 / 完整性保护卷

**目的。** 判断业界是否已有现成的开放格式能满足本仓库目标；若有则不重复造轮子，若无则在本仓库自行定义，并用文档把最终格式写下来。

**范围。** 可公开核对的 Linux 磁盘加密、机密虚机（CVM）、Confidential Containers（CoCo）盘上格式与约定。云厂商平台私有契约只在能说明「我们想要的能力」时收录，不当作可移植格式。

**方法。** 对每个候选：(1) 用统一形式化记号写出盘上布局；(2) 记录机密性 / 完整性 / 自描述 / 密钥释放属性；(3) 对照 [仓库 README](../README.md) 的需求看差距。

**日期。** 2026-09-10。

**English version:** [README.md](README.md)

---

## 1. 对照的需求

### 1.1 格式必须能表达的保护模式

「可选的机密性与完整性」本质上是在选 device-mapper 栈。先把模式列清楚，因为**哪些模式在范围内，直接决定要不要新的分区布局**（见 §1.3）。

| 模式 | device-mapper 栈 | 可写性 | 额外盘上区域 | 机密性 | 完整性 |
|------|------------------|--------|--------------|--------|--------|
| `plain` | 无 | 读写 | 无 | 无 | 无 |
| `crypt+integrity` | `dm-integrity` → `dm-crypt` | 读写 | 无（tag / journal 也在带内，`cryptsetup --integrity`） | 有 | **读写完整性** |
| `verity` | `dm-verity` | **只读** | **哈希树**（独立分区，或同设备上的 hash offset） | 无 | **只读完整性** |
| `verity+crypt` | `dm-crypt` → `dm-verity` | **只读** | 同 `verity` | 有 | **只读完整性** |

**加密就必须配完整性：这里刻意没有「只加密」的模式。** `dm-crypt` 的 AES-XTS 保长且不认证，所以能写这个设备的 host 就能改动明文——以 16 字节块为粒度，盲改但可重复——而 guest 会毫无征兆地接受这个结果。在一个「前提就是 host 能写」的威胁模型里，提供「只有机密性、没有完整性」的模式，等于提供一个看着受保护、实际不受保护的档位。所以这里每一种机密模式都配了完整性：读写配 `dm-integrity` 标签，只读配 `dm-verity`。这个蕴含是单向的——**只要完整性、不要机密性**仍然可用，那就是 `verity`，用于不可变的公开镜像。

**两种完整性都在范围内，而且它们不能互相替代。** 读写完整性完全在带内：`cryptsetup --integrity` 把 tag 和 journal 放进同一个设备，因此不需要任何新的盘上区域、也不需要新元数据。只读完整性则要两样 LUKS2 无处安放的东西：一块**第二个 extent** 放哈希树，以及一个**被认证的根哈希**（连同 salt、哈希算法、两个块大小），而它必须从「它所校验的那棵树」之外送进来。正是这个不对称决定了布局问题（§1.3），也正是它让调研里最接近的那一类失格（§6.3）。

### 1.2 需求

| ID | 需求 | 对盘上格式的含义 |
|----|------|------------------|
| R1 | 可选的机密性与完整性 | 格式必须记录 §1.1 中的哪一种模式生效，并且**打开方实际依据的就是这条记录**。 |
| R2 | 通用性 | 同一份镜像既要能被 CoCo guest（CDH / Trustee）打开，也要能被**不跑 Kata 的独立 CVM**打开。打开过程不能依赖只存在于某一个编排器上的 sidecar 文件。 |
| R3 | 自包含 | *解释*这个镜像所需的一切（模式、布局、算法、完整性配置、verity 根哈希）必须在镜像上。**不该**放在镜像上的有两样：秘密，以及**密钥定位符**。定位符是环境的属性，而且它在任何密钥可用之前就要被用到（所以永远来不及被认证），还会把一份镜像钉死在一个部署上；它由可信通道提供。镜像带的是卷的身份，由部署侧把身份映射到密钥。 |
| R4 | 开箱即用 | Linux guest 有 `cryptsetup` / device-mapper 就应能打开数据面，不必再写一套用户态密码学栈。 |
| R5 | 打开契约的真实性 | **调研之后补充的一条。** 攻击者能改写每一个字节，所以 R1/R3 里那条记录必须被认证（MAC / AEAD / 度量），且认证密钥不在 host 手里，打开路径必须消费被认证的那份副本。原生 LUKS2 不满足这条（[§4.1](#41-luks2)、[Trail of Bits](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)）。 |

R5 原本隐含在原始目标里。单独列出来，是因为各家现成格式栽的正是这一条，而不是加密本身。

**明确不在范围内：整设备的新鲜度。** 这份需求表早先有一条防重放，现在删掉了，因为没有任何盘上格式能满足它。host 可以把整个镜像恢复成一份更旧但自洽的副本，此时所有住在盘上的检查——integrity 标签、header MAC、代号计数器——全都能过，因为攻击者把证据和数据一起回滚了。要发现它必须依赖盘**外**的可变状态（一个记住每卷最新代号的密钥服务，或一个 guest 可见的单调计数器），所以它是部署的性质，不是布局的性质。[Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) 得出了同样的结论，把整快照新鲜度列为非目标，原话是「neither the disk nor Trustee records which authenticated state is newest」。因此本调研仍会记录各项目在重放上的处境作为背景，但不再把它当成一条需求来打分。

### 1.3 哪条需求逼出哪种结构

| 如果范围是…… | 结构上就需要…… | 原因 |
|--------------|----------------|------|
| 只有 `crypt+integrity`（不要只读完整性卷） | **LUKS2 之外什么都不需要** | 两种栈都在带内：LUKS2 头描述算法，`--integrity` 把 tag 和 journal 放进同一个设备。额外元数据塞进 `tokens` 就行。 |
| ……再加上 R5 | LUKS2**加一个被认证的锚点** | 这个锚点不能只是一个普通 token：头的 MAC 不可能存在它所覆盖的那个头里面。可选做法有：设备上一小块 MAC 区域（tdx-init、CDH #1648）、由可信信道给进来的分离头、或者把头的摘要绑进 KBS 的释放策略。这些都**不需要分区表**。 |
| ……再加上 `verity` / `verity+crypt` | 一块**哈希区**，实践上还要一张**分区表** | verity 哈希树是第二个对象，有自己的大小和根哈希。它可以放在同设备的 hash offset 上（`veritysetup --hash-offset`），但那样偏移就成了私有约定。用 GPT 类型 UUID 让各角色可被发现，这才是 `GPT(元数据, [verity], 数据)` 在 R2/R3 上的理由。 |

所以布局问题可以归约成两问：只读完整性是否在范围内？被认证的锚点是否需要在没有私有偏移的前提下被发现？本仓库这两问的答案都是「是」，这就是 §7 落到分区式封装、而不是只靠 token 的原因。

### 1.4 作为基线的设计意图

作为对照基线的工作设计（还不是正式规范）：

```
TargetImage  ::=  GPT( P_meta [, P_verity], P_data )

P_meta       ::=  Superblock · OpenParams · MAC_K(Superblock · OpenParams)
P_verity     ::=  VeritySuperblock    ·  HashTree          -- 可选，只读
P_data       ::=  LUKS2[ + dm-integrity ]                  -- 可写载荷

P_meta 是明文的；里面没有任何秘密，所以只认证、不加密。
它带卷的身份，但不带密钥定位符；定位符来自可信通道，
而 MAC 密钥只在证明通过之后才被释放。
CDH（或等价的 CVM agent）可以不依赖 Kata 打开镜像。
```

---

## 2. 记号

下文布局都用这套文法。`A · B` 表示拼接。`[A]` 可选。`A+` / `A*` 表示重复。未标注时大小单位是字节。偏移都相对所命名对象的起点。

```
Device            ::=  bytes[0 .. |Device|)

GPT               ::=  ProtectiveMBR · GPTHeader · PartitionEntryArray
                       · Partition+ · BackupPartitionEntryArray · BackupGPTHeader

Partition(type, label) ::=  bytes[FirstLBA .. LastLBA]

LUKS2             ::=  H_pri · J_pri · H_sec · J_sec · KeyslotArea · [Pad] · Payload

H_*               ::=  luks2_hdr_disk          -- 4096 B 二进制头
J_*               ::=  JSON_NUL · 0x00*        -- |J| = H.hdr_size - 4096
KeyslotArea       ::=  WrappedVolumeKey*
Payload           ::=  ciphertext[ segment.offset .. )

JSON              ::=  { config, keyslots, digests, segments, tokens }

IntegrityDevice   ::=  Reserved · Superblock_4KiB · Journal · (TagRun · DataRun)+
VerityDevice      ::=  [VeritySB_512] · PadToHashBlock · HashTree
```

表里用到的安全形容词：

| 术语 | 定义 |
|------|------|
| **明文元数据** | 存储侧对手能读、也能改。 |
| **带校验和的元数据** | 能发现意外损坏，挡不住恶意改写（LUKS2 头上的 SHA-256 csum）。 |
| **MAC / AEAD 元数据** | 在密钥不在存储侧手里的前提下，能发现恶意改写。 |
| **被度量的元数据** | 哈希在放密钥之前绑进 attestation / PCR / RTMR。 |
| **分离 header（detached header）** | 打开路径从受信任的内存读 header 字节，而不是从当前这块不可信设备上读（用来关掉 TOCTOU）。 |

LUKS2 规范自己写明：校验和与冗余只覆盖**随机**损坏，不覆盖故意修改（[LUKS2 spec §1.2](https://gitlab.com/cryptsetup/LUKS2-docs)）。后面几乎所有 CVM 项目都要在 LUKS2 外包一层信封，原因就在这一句。

---

## 3. 结论摘要

| 观察 | 说明 |
|------|------|
| **没有**统一的「机密持久化卷」行业盘格式 | 各项目都是 **LUKS2** 再叠自己的约定 |
| LUKS2 header **本身就是盘上元数据** | 二进制头 ×2、JSON（`keyslots`、`segments`、`tokens`…）、keyslot 区、载荷。token 是官方的「密钥从哪来」扩展点 |
| LUKS2 header **不能**抵抗 host / 存储侧对手 | 默认 AES-XTS 保长且不认证。host 能改 JSON（cipher、segment，甚至 `cipher_null-ecb`）。见 [Trail of Bits, 2025-10-30](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/) |
| 那篇研究之后的 CVM 共识 | (1) 把头拷进 guest 内存再 `cryptsetup open --header`；(2) **给 header 做 MAC** 或 **白名单校验 JSON**；可选 (3) 把清洗后的 header **度量**进 attestation |
| 最接近「自描述可移植镜像」的 | [systemd-homed](https://systemd.io/HOME_DIRECTORY/)：`GPT · LUKS2 · token(加密用户记录)`。最接近「盘上写 attestation + KBS」的：[libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs)（`tokens[i].type = "kbs"`）。两者都没有 GPT meta + verity + 被认证配置这一套 |
| 最接近产品化的 CoCo/CVM 数据盘栈 | [cryptpilot](https://github.com/openanolis/cryptpilot)：LUKS2 + 可选 `dm-integrity` + Trustee/KBS。**密钥 URI 在 guest 的 TOML 里**，不在盘上可认证对象里。FDE 的 verity root hash 在 **initrd**，不在盘上 meta 分区 |
| CoCo 持久化存储仍在收敛 | CAA CSI 把 `encrypt-type` / `kbs-key-id` 放在 **宿主机** 的 `mountInfo.json`。[guest-components #1648](https://github.com/confidential-containers/guest-components/pull/1648) / [#1663](https://github.com/confidential-containers/guest-components/pull/1663) 给 LUKS 加认证信封 + integrity。[Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) 主张把 profile 放进 **Trustee manifest**，不放盘上 |
| 新的 GPT 三分区镜像 **不是**因为缺少加密才需要 | 只要机密性，什么新东西都不需要。真正逼出它的是**只读完整性**（哈希树是第二个 extent，它的根哈希还得记在某个被认证的地方）叠加 (R2)+(R3)：这些参数必须跟着镜像走，不依赖 Kata / CSI / guest 配置。见 §6.3。调研范围内没有项目把这写成稳定规范 |

**对本仓库下一步的建议。** 数据面继续用 LUKS2 + `dm-integrity` + `dm-verity`（R4）。再规定一层**薄的、可认证的信封**（GPT `P_meta`，或 CDH 那种 header MAC 区）来装打开配置（R2、R3、R5），并且把密钥定位符彻底挪出镜像（R3）。只认证就够了：信封里没有秘密，所以这里 MAC 比 AEAD 更合适。不要试图在盘上做设备级新鲜度，也不要另起一套与 cryptsetup 不兼容的密码学载荷。

这层信封已写成 **CDF v1**：[design/README.zh.md](../design/README.zh.md)。

---

## 4. 积木格式

这些不是互相竞争的产品。后面每个项目都是它们的组合。

### 4.1 LUKS2

| | |
|---|---|
| **名称** | Linux Unified Key Setup 2 |
| **规范** | [LUKS2 On-Disk Format Specification v1.1.4 (2025-06-16)](https://gitlab.com/cryptsetup/LUKS2-docs) · [PDF 镜像](https://fossies.org/linux/cryptsetup/docs/on-disk-format-luks2.pdf) · [cryptsetup](https://gitlab.com/cryptsetup/cryptsetup) |
| **角色** | Linux 加密块设备的事实标准容器。下面几乎所有项目的载荷层。 |

**磁盘格式。**

```
LUKS2_device ::=  H_pri · J_pri · H_sec · J_sec · KeyslotArea · [Pad] · Payload

H_pri            位于字节 0
                 magic    = "LUKS" || 0xBA 0xBE
                 version  = 2
                 hdr_size = |H_pri · J_pri|          -- 16 KiB, 32 KiB, …, 4 MiB
                 seqid    : 每次写 header 单调递增
                 csum     : SHA-256(H_pri[csum:=0] · J_pri)   -- 只防意外损坏
                 uuid, label, subsystem, salt, hdr_offset

J_pri            位于字节 4096
                 |J_pri|  = hdr_size - 4096
                 内容     = 以 NUL 结尾的 JSON C 字符串，剩余填 0x00

H_sec            位于偏移 hdr_size
                 magic    = "SKUL" || 0xBA 0xBE
                 其余字段与 H_pri 同构；salt 必须不同

J_sec            J_pri 的功能副本

KeyslotArea      按需分配；不备份一份
Payload          从 segments["0"].offset 开始（常见 16 MiB）
```

允许的第二份 header 偏移（`hdr_size`）：16 KiB、32 KiB、64 KiB、128 KiB、256 KiB、512 KiB、1 MiB、2 MiB、4 MiB。

```
+------------+------------------+------------+------------------+---------------+--------+
| H_pri 4KiB | J_pri            | H_sec 4KiB | J_sec            | KeyslotArea   | Data   |
+------------+------------------+------------+------------------+---------------+--------+
0            4096               hdr_size                        keyslots.offset  segment.offset
```

**JSON 模式（五个强制顶层对象）。**

```
JSON = {
  config   : { json_size, keyslots_size, flags? },
  keyslots : { id -> Keyslot },     -- 被包裹的 volume key + KDF + AF stripes
  digests  : { id -> Digest },      -- 校验解开的 volume key 是否正确
  segments : { id -> Segment },     -- 密文区：offset, size, encryption, sector_size, integrity?
  tokens   : { id -> Token }        -- 「口令从哪来」（不透明 JSON + type）
}

Keyslot  = { type: "luks2", key_size, af, area, kdf }
Segment  = { type: "crypt" | "hw-opal" | "hw-opal-crypt",
             offset, size, encryption, sector_size, integrity? }
Token    = { type: string, keyslots: [id], ...按类型扩展的字段 }
```

**可选完整性（认证加密）。** 若设置了 `segments[i].integrity`，cryptsetup 会搭：

```
/dev/X  --dm-integrity-->  /dev/mapper/X_dif  --dm-crypt-->  /dev/mapper/X
```

生产上常见：`--cipher aes-xts-plain64 --integrity hmac-sha256`。原生 AEAD（`aes-gcm-random`、`aegis128-random`）在 cryptsetup 里仍是实验性。FDE **不建议**用 GCM（96-bit nonce）。

**能力。** 机密性（`dm-crypt`）。可选逐扇区完整性。多 keyslot。分离 header（`cryptsetup --header`）。token 扩展点。cryptsetup 2.7 起支持 OPAL 硬件加密段（`hw-opal`、`hw-opal-crypt`）。

**相对 R1–R5 的差距。** header 是明文 + 校验和，不是 MAC（R5）。规范里没有「先证明、再从 KBS 取 key」的一等公民对象。token 可以*命名*外部密钥源，但没有人认证 token 不被 host 改掉。

---

### 4.2 dm-integrity

| | |
|---|---|
| **名称** | Linux device-mapper integrity 目标 |
| **规范** | [内核文档：dm-integrity](https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html) |

**磁盘格式。**

```
IntegrityDevice ::=  ReservedSectors · Superblock · Journal · Interleaved+

Superblock      ::=  4096 B
                     magic, version,
                     log2(interleave_sectors),
                     tag_size,
                     journal_section_count,
                     provided_data_sectors

Journal         ::=  JournalSection+
JournalSection  ::=  Metadata_4KiB · DataArea
                     Metadata 含 {logical_sector, tail_8B, tag}*
                     外加 8 B 的扇区号 MAC 和 commit id
                     （这是日志原子性，不是用户数据的密码学 MAC，
                      除非另外配置了 integrity key）

Interleaved     ::=  TagRun · DataRun
TagRun          ::=  tags[n], |TagRun| ≥ 4 KiB
DataRun         ::=  data_sectors[2^log2_interleave]
```

单独用 `dm-integrity`（不配 LUKS）可以用 CRC 抓静默损坏。那**不是**认证。密码学模式（`hmac-sha256`，或经 LUKS2 的 AEAD）需要密钥。

**能力。** 对**可变**数据做逐扇区篡改证据。日志保证 data+tag 原子更新。**不能**单独防止重放一张「内部自洽的旧快照」（host 整盘换掉）。

**说明。** 完整性 ≠ 新鲜度。这是真实的局限，但按 §1.2 它不在格式范围内：要补上必须依赖盘外状态。

---

### 4.3 dm-verity

| | |
|---|---|
| **名称** | Linux device-mapper verity 目标 |
| **规范** | [内核文档：dm-verity](https://docs.kernel.org/admin-guide/device-mapper/verity.html) · [cryptsetup DMVerity wiki](https://gitlab.com/cryptsetup/cryptsetup/-/wikis/DMVerity) |

**磁盘格式**（用户态约定；内核**不解析** superblock，只消费哈希块 + 从可信通道送来的 root hash）：

```
VerityDevice ::=  [SB] · PadTo(hash_block_size) · HashTree

SB ::= 512 B
     signature        = "verity\0\0"
     version          = 1
     hash_type        = 0 (Chrome OS) | 1 (现行)
     uuid
     algorithm[32]    -- 例如 "sha256"
     data_block_size, hash_block_size
     data_blocks
     salt_size, salt[256]

HashTree ::= 深度优先，根在前，下标递增
             leaf  = H(salt || data_block)     -- type 1：salt 前置
             node  = H(salt || child_hashes)
```

**能力。** 静态树的只读完整性 / 真实性，**当且仅当** root hash 可信（initrd、被度量的配置，或被认证的 meta 分区）。适合不可变镜像。

**相对 R3 的差距。** root hash 不能自己证明自己。必须有一条可信通道带着它。cryptpilot FDE 放在 initrd；本仓库的目标设计会放进 `P_meta`。

---

### 4.4 GPT（发现层）

GUID 分区表不是密码学格式。它只是让上面这些对象在同一张镜像上**可被发现**的常用办法。

```
Image ::= GPT(
            P_i  where P_i = LUKS2 | VerityDevice | IntegrityDevice | Filesystem | Opaque
          )
```

靠 **type UUID** 和 **分区 label** 发现。systemd-homed 是「这个 GPT 类型就等于这份 LUKS+token 契约」最干净的先例（M 类，§5.4）。新的机密卷格式应为 `P_meta` / `P_verity` / `P_data` 分配自己的 type UUID。

---

## 5. 按磁盘格式分类

有意义的分类轴不是项目，而是**打开契约放在哪里、有没有被认证**。调研到的全部东西落在六类里。项目作为某一类的子项出现；少数项目跨两类，所以出现两次。

| 类 | 形态 | 相比 LUKS2 多了什么 | 成员 |
|----|------|--------------------|------|
| **L** | `Device ::= LUKS2`，契约在头里（`tokens`） | 什么都没多 | Clevis/Tang、systemd-cryptenroll、libcryptsetup-token-kbs、systemd-homed、Constellation、tdx-init（SSH key） |
| **C** | `Device ::= LUKS2`，契约**不在镜像上** | 什么都没多 | cryptpilot-crypt、Contrast、Cosmian、dstack、Oasis、Secret Network、CAA CSI、Kata #13638 |
| **E** | `Device ::= Envelope · Payload`，同设备上有被认证的区域 | MAC、分离头、崩溃状态 | tdx-init v0.2.0、CDH #1648/#1663、Fortanix Salmiac |
| **M** | `Image ::= GPT(各角色…)`，角色写在分区表里 | 可发现性，以及放 verity 树的地方 | systemd-homed 镜像、cryptpilot FDE、**CDF v1（本仓库）** |
| **P** | 平台契约，不是可移植镜像 | 不适用 | Azure、GCP、AWS / Nitro、IBM SE、qcow2+LUKS |
| **X** | 非 LUKS 的整盘格式 | 不适用——数据面不同 | VeraCrypt、BitLocker、TCG Opal、ZFS / fscrypt / ocicrypt |

L 类和 C 类**盘上字节完全相同**，区别只在于打开配置是否跟着镜像走，而且 `blkid` 看不出来。注意 L 类的 `tokens` 通常还带一个密钥定位符；按 §1.2 现在的 R3，那部分是多余而非必需，而且因为它没被认证，反而是个负担。

### 5.1 L 类 —— 原生 LUKS2，打开契约在 LUKS2 头里

**格式。** 未经改动的 LUKS2（§4.1）。契约是一个或多个 `tokens[i]`；cryptsetup 原样存这段 JSON，不认识的类型就忽略。

```
Device     ::=  LUKS2
tokens[i]  ::=  { type: <string>, keyslots: [k], ...各类型自有字段 }

-- JSON 区是明文，唯一保护是无密钥的 SHA-256 csum，
-- 因此 host 只要把 csum 一起改对，改写就会被接受
```

| 成员 | `tokens[i].type` | token 里装了什么 | 备注 |
|------|------------------|------------------|------|
| [Clevis / Tang](https://github.com/latchset/clevis) | `clevis` | 每个 pin 一个 Jose 对象：Tang ads、TPM2 密封块、SSS 门限 | 网络 / TPM 解锁；假设有个公证人，而不是 TEE |
| [systemd-cryptenroll](https://www.freedesktop.org/software/systemd/man/latest/systemd-cryptenroll.html) | `systemd-tpm2`、`systemd-fido2`、`systemd-pkcs11`、`systemd-recovery` | 密封块或公钥 + PCR 策略 | 设计上只支持 LUKS2，因为 JSON token 区**就是**存储 |
| [systemd-homed](https://systemd.io/HOME_DIRECTORY/) | `systemd-homed` | `record` = Base64(Enc_VK(用户记录))、`iv` | 用同一个卷密钥加密，IV 不同；同时属于 M 类 |
| [Constellation](https://docs.edgeless.systems/constellation/architecture/encrypted-storage) | `constellation-state-disk` | `diskIsInitialized` 等 | DEK 来自集群 KeyService；[GHSA-hq76-6gh2-5g4q](https://github.com/edgelesssys/constellation/security/advisories/GHSA-hq76-6gh2-5g4q) 之后改为分离头 + JSON 白名单打开 |
| [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs) | `kbs` | KBS URL + resource path，或指向 SMBIOS 里的 `initdata.toml` | 现存最接近「先证明再解锁」的盘上契约；需要装插件 |
| [tdx-init](https://github.com/flashbots/tdx-init) | 头里的元数据，不是 token | searcher SSH key | 让首次启动自描述；它的 MAC 扇区属于 E 类 |

**性质。** 加密参数这一项 R3 成立，因为它们就在 LUKS2 头里；但这里没有任何地方能记 verity 根。R4 完美：任何 `cryptsetup` 都能打开。R1 记在 `segments` 里，但见 R5。**R5 对每个成员都不成立**——token 和 segment 算法都可被 host 改写，也就是说这些项目依赖的那个盘上定位符，恰好是攻击者能改指向的字段。

**这一类决定了布局问题的答案。** 如果范围只有机密性，L 类**就是**答案，不需要新格式（§1.3 第一行）。它只在加上 R5 之后才不够用。

---

### 5.2 C 类 —— 原生 LUKS2，打开契约不在镜像上

**格式。** 盘上字节和 L 类逐字节相同（往往连 token 都没有）。用哪个密钥、哪种模式、只读还是读写，这些配置在别处，所以单看这块盘不知道怎么打开。

```
Device       ::=  LUKS2                       -- 与 L 类无从区分
OpenProfile  ::=  <在镜像之外>                 -- guest 配置 | 集群状态
                                              -- | host CSI 元数据 | 被度量的 manifest
```

| 成员 | 契约放在哪 | 备注 |
|------|-----------|------|
| [cryptpilot-crypt](https://github.com/openanolis/cryptpilot/blob/master/cryptpilot-crypt/docs/configuration.md) | guest 里的 `/etc/cryptpilot/volumes/*.toml`：`dev`、`encrypt.kbs.url`、`encrypt.kbs.resource_path` | LUKS2 `aes-xts-plain64`，`integrity=true` 时用 `hmac-sha256`；这一类里 KBS 故事最完整的 |
| [Contrast](https://github.com/edgelesssys/contrast/security/advisories/GHSA-f5p4-p5q5-jv3h) | 集群 secret seed | v1.12.1 带 cryptsetup ≥ 2.8.1；v1.13.0 加了 tmpfs 分离头、JSON 校验、可选 `dm-integrity`（#1734） |
| [Cosmian VM](https://github.com/Cosmian/cosmian_vm/pull/206) | TPM / vTPM 绑定；文件容器 `/var/lib/cosmian_vm/container` | 正在转向分离头 + `MAC_K(Header)` + `dm-integrity` |
| [dstack](https://github.com/Dstack-TEE/dstack/security/advisories/GHSA-jxq2-hpw3-m5wf) | `/data` 对应的 guest / CVM 状态 | 现在会把 header 备份到 tmpfs、白名单校验 JSON、用 `--header` 打开；advisory 建议给整个 header 做 MAC |
| [Oasis ROFL](https://github.com/oasisprotocol/oasis-sdk)、[Secret Network](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/) | 各项目自有 | 同一类补救：白名单 + 分离头 |
| [CAA CSI](https://confidentialcontainers.org/blog/2026/08/14/encrypted-persistent-storage-for-peer-pods-with-the-caa-csi-block-driver/) | **host** 上的 `mountInfo.json`：`encrypt-type`、`kbs-key-id` | 卷本身从不携带 `kbs-key-id`；guest 侧判定是 `isLuks()` 看 6 个魔数字节 |
| [Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) | Trustee manifest + 被度量的 Agent policy | 有意把配置放在盘外；整快照新鲜度是它明确的非目标（§1.2） |

这一类反复出现同一个 bug：「看起来像 LUKS 就重开，否则 format」。Contrast 和 CDH 都不得不换掉这套启发式，因为 host 递进来一个头，就能把「打开失败」变成「重新格式化」。

**性质。** R1、R4 成立。**R2、R3 不成立**：把盘交给一台没有 guest TOML、没有集群 seed、没有 Kata policy 的 CVM，就没有任何东西能打开它。R5 是在打开流程里补的，不是格式给的。

---

### 5.3 E 类 —— LUKS2 之外，同设备上加一块被认证的区域

**格式。** 设备不再是「纯 LUKS2」。在实现约定好的固定偏移上有一小块区域，用来认证头；LUKS 载荷相应挪位或整体分离。

```
Device    ::=  Envelope · Payload

Envelope  ::=  MAC_K( Header )            -- 密钥 K 不在 host 手里
             [ · DetachedHeader ]         -- 权威元数据，不走现场盘上路径
             [ · CrashState ]             -- 代号 / journal，用于 fail-closed 重开

Open      ::=  在 guest 内存里校验 Envelope
               cryptsetup open --header H_ram Payload
```

| 成员 | 布局 | 细节 |
|------|------|------|
| [tdx-init v0.2.0](https://github.com/flashbots/tdx-init/releases/tag/v0.2.0) | `MACSector_512 · LUKS2`——载荷向前挪一个 512 B 扇区 | ToB 引作「给 header 做 MAC」的参考实现。证明几百字节前导就够关掉这个 bug。MAC 密钥是运营口令，不是证明后释放的密钥 |
| [CDH #1648](https://github.com/confidential-containers/guest-components/pull/1648) / [#1663](https://github.com/confidential-containers/guest-components/pull/1663) | `Region(分离 LUKS2 头, MAC_K(Header), CrashState) · Payload` | 全零才初始化、fail-closed 重开（绝不因为打开失败就格式化）、带 journal 的 `hmac-sha256` `dm-integrity`。CoCo 主干里的答案 |
| [Fortanix Salmiac](https://github.com/fortanix/salmiac) | MAC + 分离头 | 同一类补救，未公开布局 |

```
+-----------+----------------------------------------------+
| MAC 512 B | LUKS2（header + keyslots + payload）         |   tdx-init
+-----------+----------------------------------------------+
0           512
```

**性质。** **R5 成立**——只有这一类靠自己就认证了打开契约。R4 成立：载荷仍由 `cryptsetup` 打开。CDH 的崩溃状态额外让重开变成 fail-closed，那解决的是崩溃一致性，不是新鲜度。**R2/R3 仍弱**：区域在私有偏移上、没有类型 UUID，通用 agent 发现不了。

**为什么不能直接拿来用。** 这一类把 `crypt+integrity` 覆盖得很完整——读写完整性在带内，而 CDH #1663 的带日志 `hmac-sha256` 正是本仓库最终采用的做法。但它对 **`verity` 和 `verity+crypt` 完全没有覆盖**：信封的定义就是「对一个 *LUKS2 头* 做 MAC」，而 LUKS2 头既没有放 verity 根哈希的字段，也没有放哈希树的第二个 extent；三个成员里没有任何一个定义了这些。要补上，就得引入「可被发现的 extent」和「一个被认证的地方来记它们的几何参数」，那就是 M 类。完整论证见 §6.3。

---

### 5.4 M 类 —— 多分区镜像，角色写在分区表里

**格式。** 盘上有多个角色，靠分区表让它们可被发现。只有这一类能把 `dm-verity` 哈希树当一等对象来放，因为哈希树是第二块区域，有自己的大小和根哈希。

```
Image  ::=  GPT( P_1, P_2, … )
P_i    ::=  LUKS2 | VerityDevice | IntegrityDevice | Filesystem | MetaRegion

靠类型 UUID + 分区标签发现——不需要插件，不需要私有偏移。
```

**成员：[systemd-homed](https://systemd.io/HOME_DIRECTORY/)**（[源码文档](https://github.com/systemd/systemd/blob/master/docs/HOME_DIRECTORY.md)）—— 规范写法正是我们想要的那种先例。

```
HomeImage ::=  GPT( P_home )

P_home.type_uuid  = 773f91ef-66d4-49b5-bd83-d683bf40ad16
P_home.label      = Username

P_home            ::=  LUKS2                       -- label = Username
LUKS2.tokens[i]   ::=  { type:   "systemd-homed",
                         record: Base64( Enc_VK(UserRecordJSON, IV) ),
                         iv:     Base64(IV) }
LUKS2.Payload     ::=  Filesystem ∈ { ext4, btrfs, xfs }
```

一个文件或移动盘就是完整、自描述的家目录：身份、怎么解锁、怎么校验用户记录。先解锁再信任文件系统。但元数据是*桌面用户记录*，不是 attestation + KBS；没有 verity 分区；header 仍可被 host 改（L 类的老问题）。

**成员：[cryptpilot](https://github.com/openanolis/cryptpilot) FDE** —— 多卷，但锚点在盘外。

```
FDE_disk ::=  GPT( P_rootfs, P_delta, … )

P_rootfs ::=  [LUKS2] · VerityProtectedRootfs      -- 只读根，必须 dm-verity
P_delta  ::=  LUKS2[ integrity = hmac-sha256? ]    -- 必须加密；overlayfs / COW

Anchor   ::=  metadata.toml ⊂ initrd               -- 含 verity root_hash
              跟启动链一起被度量，不是盘上被认证的区域

栈：     P_rootfs → [dm-crypt] → dm-verity → … → dm-snapshot → /sysroot
         P_delta  → dm-crypt   → [dm-integrity] → COW
```

最接近产品化的「CVM + 数据盘 + KBS」栈（[FDE boot](https://github.com/openanolis/cryptpilot/blob/master/cryptpilot-fde/docs/boot.md)、[FDE config](https://github.com/openanolis/cryptpilot/blob/master/cryptpilot-fde/docs/configuration.md)、[阿里云说明](https://www.alibabacloud.com/help/en/ecs/user-guide/disk-measurement-and-encryption-protection-for-confidential-computing-instances)）。**R3 不成立**：verity 根在 initrd 里，镜像无法描述自己的只读完整性。（密钥 URI 在 guest TOML 里**不是**失分点——被度量的 guest 配置本来就是合法的定位符通道，§1.2 的 R3 还更倾向它而不是盘上字段。）

**成员：CDF v1（本仓库）** —— `GPT(P_meta, [P_verity], P_data)`，打开契约以明文存在 `P_meta` 里、外加一个 MAC。见 [design/README.md](../design/README.md)。

**性质。** R2/R3 只有在这一类里可达，因为角色不靠私有偏移就能发现，且有地方放 verity 根。R5 **不是自动的**：homed 和 cryptpilot 都在这一类，两者都不满足。分区表买到的是可发现性，不是真实性——真实性仍要靠 E 类的机制。

---

### 5.5 P 类 —— 平台契约，不是可移植镜像

**格式。** 盘上布局是云厂商私有的，契约是一套 API 加平台的密钥释放策略。列出来只为对照能力：这些都不能把盘交给另一台 CVM 当自描述卷用，所以作为格式都不满足 R2/R3。

```
Device   ::=  <平台加密的块设备>              -- 布局未公开，或不可移植
Contract ::=  云 API + 与证明绑定的平台密钥
```

| 平台 | 盘上有什么 | 为什么不是候选格式 |
|------|------------|--------------------|
| [Azure 机密 VM](https://learn.microsoft.com/en-us/azure/confidential-computing/confidential-vm-overview) | `OSDisk_encrypted_by_platform · VMGS · [guest LUKS \| BitLocker]`。VMGS 是一块小的伴随盘，存 vTPM / UEFI 状态；密钥释放通过 `SecurityEncryptionType ∈ {DiskWithVMGuestState, …}` 绑到证明上 | 整个 P 类里能力最接近的一个——host 读不到 OS 盘明文——但布局是云厂商私有，而且 vTPM 锚点在**另一块盘**上，不在卷里。guest 若再叠自己的 LUKS，就把 §4.1 的所有问题一并继承 |
| Google Compute CSEK / CMEK | 平台包裹的 DEK；guest 看到普通块设备 | 密钥契约是 GCP API，不是镜像头 |
| AWS EBS / Nitro Enclaves | EBS：平台 AES。Nitro：不挂 EBS，秘密走带证明的 KMS。[enclavia-crypto](https://github.com/EnclaviaIO/enclavia) 在 LUKS 文件前加 4 KiB RSA-OAEP key blob | 那 4 KiB 是临时信封，无公开规范 |
| [IBM Secure Execution](https://www.ibm.com/docs/en/linuxonibm/pdf/lx24se04.pdf) | guest 数据是普通 LUKS；口令烧进加密的 SE 镜像 / initrd 和 `/etc/crypttab` | 锚点是 SE header + host-key document，不在数据盘 |
| [QEMU qcow2 + LUKS](https://github.com/qemu/qemu/blob/master/docs/interop/qcow2.rst) | 头扩展 `0x0537be77` 指向 qcow2 文件内的 LUKS1 头；cluster 载荷加密，qcow2 元数据明文 | 是虚拟镜像容器而非可直接挂载的 raw 卷；LUKS1，没有 `dm-integrity` |

其中两个仍有参考价值：qcow2 和 enclavia 都是**一小段前导 + 标准 LUKS 载荷**，和 tdx-init 的 MAC 扇区、GPT 元数据分区是同一形状。Azure 的 VMGS 则说明把锚点放到第二块设备上的代价——它不跟着卷走。

---

### 5.6 X 类 —— 非 LUKS 的整盘格式，不用作底座

每个一行：格式是什么，为什么不当底座。

| 格式 | 不用的原因 |
|------|-----------|
| [VeraCrypt](https://veracrypt.io/en/VeraCrypt%20Volume%20Format%20Specification.html) —— `Salt_64 · Enc_K(Header_448 · MasterKeys) · Reserved · Payload [· 64 KiB 处的隐藏头]` | 与 `cryptsetup` 不兼容：没有 LUKS token，不能叠 `dm-integrity`。它的可否认目标（整盘看起来像随机数）直接和 R3 冲突，而 R3 要的是元数据可被发现。 |
| [BitLocker / FVE](https://github.com/libyal/libbde/blob/master/documentation/BitLocker%20Drive%20Encryption%20%28BDE%29%20format.asciidoc) —— `VolumeHeader("-FVE-FS-") · 3 × FVE_MetadataBlock · EncryptedPayload`，VMK 被 protector 包裹、FVEK 被 VMK 包裹 | 与 `cryptsetup` 不兼容，数据面只在 Windows。值得借的想法——盘上密钥保护器由硬件解封——LUKS2 token + TPM2 / KBS 已经在做。 |
| [经 LUKS2 使用的 TCG Opal](https://gitlab.com/cryptsetup/LUKS2-docs) —— `segments[i].type ∈ {hw-opal, hw-opal-crypt}`、`requirement "opal"` | 是硬件锁定范围而非镜像格式，且不可信 host 模型不信任磁盘固件。当可选 segment 类型可以，当契约不行。 |
| ZFS 原生加密、fscrypt、fs-verity、ocicrypt | 层次不对：给不出 CDH 能 `SecureMount`、CVM 能 `cryptsetup open` 的块设备。 |

---

## 6. 对照表

意图再述：`GPT(P_meta_MAC, [P_verity], P_data_LUKS2[+integrity])`；密钥在 Trustee；定位符来自可信通道；CDH 或 CVM agent 不依赖 Kata 打开。

**「LUKS2 头可替换」列**（Trail of Bits / CVM host）。Host 能改明文 LUKS2 头。若 `open` **失败**，通常只是可用性损坏，旧 AES-XTS 密文仍机密。若 `open` **成功**但数据面被改弱（或「打开成功却 mount 不上」之后走了 format），**之后的新写入**可以对 host 可见。这不只发生在 from-scratch 空盘上。

| 单元格 | 含义 |
|--------|------|
| **有** | 风险在：现场打开信任盘上头；没有公开的 MAC / 白名单 / 度量。 |
| **减轻：分离 + 白名单** | ToB 方案 2。常见算法降级和 TOCTOU 能挡住，前提是校验和 `open` 用同一份内存副本。残留：没列进白名单的字段（reencrypt、新 token）。 |
| **已缓解：MAC + 分离** | ToB 更推荐的。头字节用 host 没有的密钥认证；`open --header` 用这份副本。 |
| **不适用** | 不是 LUKS2 打开契约（信任根不同）。 |

### 6.1 按类对照

这些性质是类的性质，所以这张才是关键的表。§1.1 的两种完整性拆成两列，因为各类真正分道扬镳的地方就在这里；最后一列是裁决。

| 类 | 读写完整性 | 只读完整性（`dm-verity`） | 打开契约在镜像上？ | 可移植（R2/R3） | 契约被认证（R5） | **为什么不能直接用** |
|----|------------|--------------------------|--------------------|-----------------|------------------|----------------------|
| **L** 原生 LUKS2，契约在头里 | 有 —— 带内 `dm-integrity` | **没有** —— LUKS2 既无记根哈希的字段，也无放哈希树的第二个 extent | 在 —— header / `tokens`，但**未认证** | 部分：需要插件或已知 token 类型 | **不成立** —— SHA-256 csum ≠ MAC | 两处独立的失败：契约可被 host 改写（R5），而且只读完整性根本无法表达（R1） |
| **C** 原生 LUKS2，契约在盘外 | 有 | **没有** —— 同 L | **不在** | **不成立** —— 配置不跟着盘走 | 在打开流程里补（分离 + 白名单） | 镜像不自描述（R2/R3），而且只读完整性仍然无处安放（R1） |
| **E** LUKS2 + 被认证区域 | 有 —— CDH #1663 带日志 `hmac-sha256` | **没有** —— 信封认证的是*一个 LUKS2 头*；没有根哈希字段，也没有约定的哈希树区域 | 在，但在**私有偏移**上 | 弱：无类型 UUID，发现不了 | **成立** | **只读完整性做不到**（R1），区域也发现不了（R2/R3）。两者的修法都要一张角色图——那就是 M 类。见 §6.3 |
| **M** 多分区镜像 | 有 | **有** —— 可以拿一个分区放哈希树，根哈希放进 `P_meta` | 在，且**可被发现**；只有 M + E 才被认证 | 成立 | 只有叠加 E 才成立 | 结构上没缺什么，但已发布的成员里契约都未被认证（R5），所以单靠 M 同样不能用 |
| **P** 平台契约 | 平台 | 平台 | 不适用 —— 在云 API | 不成立 | 平台内部 | 它根本不是一种可移植的镜像格式（R2） |
| **X** 非 LUKS 格式 | 各异 | 各异 | 在（自有头） | 过不了 R4 | 各异 | 数据面就不对：原生 `cryptsetup` 打不开（R4） |

没有任何一类提供设备级新鲜度，见 §1.2。

### 6.2 按项目对照

顺序同 §5：

| 项目 | 类 | 完整性 | 密钥 / 策略位置 | LUKS2 头可替换（ToB）？ |
|------|----|--------|-----------------|-------------------------|
| LUKS2（积木） | — | 可选 integrity | 口令 / token | **有** —— SHA-256 csum ≠ MAC |
| dm-verity（积木） | — | 只读树 | root hash 走可信通道 | **不适用** —— 信任根是 root hash，不是 LUKS 头 |
| Clevis / cryptenroll | L | 继承 LUKS | Tang / TPM / FIDO2 | 在 CVM host 下**有** |
| systemd-homed | L + M | 仅 LUKS | 用户口令 | 在 CVM host 下**有**（原本不是这个威胁模型） |
| Constellation | L | 可选 | 集群 KeyService | **减轻：分离 + 白名单**（advisory 之后） |
| libcryptsetup-token-kbs | L | 继承 LUKS | KBS，经 token 或 SMBIOS | **有** —— 插件只负责取 key，头仍未认证 |
| cryptpilot-crypt | C | 可选 integrity | **guest TOML** + KBS URI | **有** —— 打开信任盘上头 |
| Contrast | C | 可选（v1.13） | 集群 secret seed | **减轻：分离 + 白名单**（v1.13） |
| Cosmian / dstack | C | 加固中 | TPM / 本地状态 | **减轻** —— Cosmian：MAC 路径；dstack：白名单 + 分离 |
| Oasis / Secret Network | C | JSON 检查 | 项目私有 | **减轻：分离 + 白名单** |
| CAA CSI | C | 未规定 | host `mountInfo.json` 里的 KBS id | **有** —— `isLuks` 只看 6 字节；除非走 #1648，CDH 打开仍信盘上头 |
| Kata #13638 | C | 经 CDH | 被度量策略 + Trustee manifest | **若使用 CDH 信封则已缓解**；manifest 本身不给头做 MAC |
| tdx-init | E | header MAC | 运营口令 | **已缓解：MAC + 分离** |
| CDH #1648 / #1663 | E | header MAC + 可选 integrity | KBS | **已缓解：MAC + 分离**，外加 fail-closed 重开 |
| Salmiac | E | MAC | 项目私有 | **减轻 / 已缓解** —— 有 MAC，未公开布局 |
| cryptpilot FDE | M | verity + 可选 integrity | KBS + verity root 在 **initrd** | 其 LUKS 卷上**有**；verity 路径是另一类问题 |
| Azure Conf. OS disk | P | 平台 | AKV + attestation | 平台盘**不适用**；guest 再叠 LUKS2 则**有** |
| **CDF v1（本仓库）** | **M + E** | 可选 verity + integrity | 定位符来自可信通道；**镜像上不放定位符** | **已缓解** —— 打开契约做 MAC + 分离头 `open` |

### 6.3 为什么最接近的两类仍然不能直接采用

六类里有四类可以很快排除：**P** 不是可移植镜像，**X** 数据面就不对（R4），**L** / **C** 无法认证打开契约（R5）——这一条正是 Trail of Bits 的结论。真正值得说的是 **E** 和 **M**，因为它们各自解决了对方缺的那一半，而且看上去都只差一小步。

**E 类**是唯一靠自己就满足 R5 的类，所以它天然是「直接用这个吧」的候选。它栽在**只读完整性**上：

1. **信封的定义是「对一个 LUKS2 头做 MAC」，而 LUKS2 头描述不了一个 verity 卷。** 只读完整性带来两个无处安放的对象：哈希树，它是有自己大小的第二个 extent；以及根哈希连同 salt、哈希算法、两个块大小，它们必须从「它所校验的那棵树」之外被认证地送进来。tdx-init、CDH #1648/#1663、Salmiac 都没有为其中任何一样定义字段——它们设计出来是为了修头伪造，不是为了描述第二种保护模式。
2. **`veritysetup --hash-offset` 不算出路。** 哈希树技术上可以放在同设备的某个偏移上，但那样格式里就同时有「信封的私有偏移」和「哈希树的私有偏移」，而且仍然没有一个被认证的地方来记根哈希。等于把一个发现不了的约定变成两个。
3. **它的读写那一半是好的，而且是被采纳而不是被否决的。** CDH #1663 的带日志 `hmac-sha256` 正是本仓库为 `crypt+integrity` 规定的做法。缺口恰好就是 §1.1 里只读那一半——也就是说，让这一类失格的是「把只读完整性纳入范围」，而不是任何跟加密有关的事。
4. **信封本身也发现不了。** 固定偏移、没有类型 UUID，意味着通用 agent 区分不出「带信封的设备」和「坏掉的 LUKS2 设备」，这就是 §5.2 那个「像 LUKS 就开、否则就 format」的坑，而且更锋利。

第 1 点和第 4 点的修法是同一件事：让 extent 的角色可被发现，并且有一个被认证的地方记它们的参数。这正是 M 类的定义。所以 E 类无法在「不变成 M + E」的前提下被*扩展*成答案。

**M 类**正好是镜像对称的情况。它有地方放哈希树、也有地方放根哈希，所以能表达 §1.1 里的每一种模式——但分区表只买到发现能力，买不到真实性。两个已发布的成员都印证了这点：systemd-homed 的威胁模型里没有恶意 host；cryptpilot FDE 把 verity 根放在 initrd 而不是镜像上某个被认证的对象里，于是恰好在我们讨论的这个模式上不满足 R3。

**于是结论。** 只读完整性逼出 M，R5 逼出 E，而调研范围内没有任何东西同时是两者。因此 CDF v1 被规定为 M + E：一张可被发现的角色图，其元数据分区用 host 拿不到的密钥做认证。

---

## 7. 对即将落盘的格式的启示

1. **不要替换 LUKS2。** 认真的 Linux CVM/CoCo 栈都把 LUKS2 当密文容器，好让 `cryptsetup` 继续当数据面（R4）。新规范应是 **信封 + 发现层** —— 用 §5 的说法，就是 **M 类（可发现）叠加 E 类（可认证）**。调研到的项目里没有一个同时做到这两点。

2. **「元数据在盘上」已有现成做法；「可认证的元数据在盘上」还没有。** token（homed、Clevis、cryptenroll、Constellation、`kbs`）解决可发现性。它们**不**解决恶意 host 改 JSON。本仓库要发布的格式必须指定下面之一：
   - `MAC_K(header)`（tdx-init、Salmiac、CDH #1648），或
   - `AEAD_K(meta)`（加密的 `P_meta`），或
   - 放密钥之前把清洗后的 header 度量进 attestation，
   并且打开时必须走 **guest 内存里的分离 header**。

   前两者之间，**MAC 更合适**，本仓库也就这么定。威胁是 host 改写打开参数，不是 host 读到它们：算法、完整性配置、verity 根，乃至 LUKS2 头本身，在这些原语的其他任何部署里都是公开的。把它们加密会带来 nonce 管理，会迫使格式再放一份明文对象好让「没有密钥时」也能认出这个卷，还会让盘对恢复工具变成不可读——而换来的机密性本来就不需要。现有的两个 E 类实现都得出了同样的答案。见 [design §7.1](../design/README.md#71-为什么用-mac而不用-aead)。

3. **用 `kbs` token 来对照，才能说清为什么还要 `P_meta`。** 类型为 `kbs` 的 LUKS token 加上 header MAC，对「装了插件的 CVM」已经能回答「密钥在哪」。独立 GPT meta 分区只有在同一份可认证对象里还能装下这些东西时才值得标准化：
   - 分区图 / 角色（`data`，可选 `verity`），
   - 完整性配置（`none | hmac-sha256 | …`），
   - verity 根哈希——这东西在 LUKS token 里根本无处安放，
   - 足够可读的元数据，让通用 agent 不加载 cryptsetup 插件也能判断「这是机密卷」。

   注意这份清单里刻意**没有**密钥定位符。与 `kbs` token 不同，`P_meta` 不指名任何密钥服务。定位符在被需要的那一刻无法被认证，而且尝试过的项目都在往同一个方向走——`kbs` token 插件自己就更倾向 SMBIOS 的 `initdata.toml` 而不是盘上字段，Kata #13638 则把 manifest URI 放进被度量的 policy。见 [design §6.2](../design/README.md#62-密钥获取走带外)。

4. **完整性分两层。**
   - 不可变 / 只读：`dm-verity`，root hash 放在可认证 meta（不要只放 initrd）。
   - 可变 / 读写：`dm-integrity`（hmac-sha256）。它能发现*扇区被换*，不能发现*整盘被换成一张旧的好盘*。

5. **把「设备新鲜度不解决」明说出来。** host 做快照回滚，integrity 标签和 header MAC 都会全部通过，因为攻击者把证据和数据一起回滚了。格式应当把这条写成非目标，而不是用一个同样被攻击者掌握的盘上计数器暗示有保护。确实需要的部署在格式之上自己加外部锚点。

6. **能复用的信封就复用。** CDH #1648/#1663（fail-closed 重开、header MAC、integrity）和 tdx-init（MAC 前导）应看成信封的实现证明，而不是竞争格式。本仓库要写的文档是那些实现都能生成、都能消费的**稳定、不绑 Kata 的布局**。

7. **发现用 UUID。** 学 systemd-homed：分配 GPT type UUID 以及 LUKS `subsystem` / `label`，让从没听说过 CoCo 的 CVM 也能用 `blkid` / udev 在 raw 盘上认出这张镜像。

---

## 8. 参考链接

**规范**

- [LUKS2 On-Disk Format Specification](https://gitlab.com/cryptsetup/LUKS2-docs)（[PDF](https://fossies.org/linux/cryptsetup/docs/on-disk-format-luks2.pdf)）
- [LUKS1 On-Disk Format](https://gitlab.com/cryptsetup/cryptsetup/-/wikis/LUKS-standard/on-disk-format.pdf)
- [dm-integrity](https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html)
- [dm-verity](https://docs.kernel.org/admin-guide/device-mapper/verity.html) · [cryptsetup DMVerity wiki](https://gitlab.com/cryptsetup/cryptsetup/-/wikis/DMVerity)
- [QEMU qcow2（LUKS 扩展）](https://github.com/qemu/qemu/blob/master/docs/interop/qcow2.rst)
- [VeraCrypt Volume Format Specification](https://veracrypt.io/en/VeraCrypt%20Volume%20Format%20Specification.html)
- [BitLocker (BDE) format（libbde）](https://github.com/libyal/libbde/blob/master/documentation/BitLocker%20Drive%20Encryption%20%28BDE%29%20format.asciidoc)
- [systemd Home Directories](https://systemd.io/HOME_DIRECTORY/)
- [TCG Opal / LUKS2 hw-opal](https://gitlab.com/cryptsetup/LUKS2-docs)

**CVM / CoCo 实现**

- [openanolis/cryptpilot](https://github.com/openanolis/cryptpilot)
- [阿里云：机密磁盘度量与加密](https://www.alibabacloud.com/help/en/ecs/user-guide/disk-measurement-and-encryption-protection-for-confidential-computing-instances)
- [Edgeless Constellation encrypted storage](https://docs.edgeless.systems/constellation/architecture/encrypted-storage)
- [Edgeless Contrast GHSA-f5p4-p5q5-jv3h](https://github.com/edgelesssys/contrast/security/advisories/GHSA-f5p4-p5q5-jv3h)
- [Cosmian/cosmian_vm](https://github.com/Cosmian/cosmian_vm)
- [Dstack-TEE/dstack GHSA-jxq2-hpw3-m5wf](https://github.com/Dstack-TEE/dstack/security/advisories/GHSA-jxq2-hpw3-m5wf)
- [flashbots/tdx-init](https://github.com/flashbots/tdx-init)
- [latchset/clevis](https://github.com/latchset/clevis)
- [systemd-cryptenroll](https://www.freedesktop.org/software/systemd/man/latest/systemd-cryptenroll.html)
- [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs)
- [CoCo CAA CSI 加密持久存储](https://confidentialcontainers.org/blog/2026/08/14/encrypted-persistent-storage-for-peer-pods-with-the-caa-csi-block-driver/)
- [Kata #13638 confidential persistent block storage](https://github.com/kata-containers/kata-containers/issues/13638)
- [guest-components#1648](https://github.com/confidential-containers/guest-components/pull/1648) · [#1663](https://github.com/confidential-containers/guest-components/pull/1663) · [#1691](https://github.com/confidential-containers/guest-components/pull/1691)
- [Azure Confidential VM](https://learn.microsoft.com/en-us/azure/confidential-computing/confidential-vm-overview)
- [IBM Secure Execution](https://www.ibm.com/docs/en/linuxonibm/pdf/lx24se04.pdf)

**攻击与缓解（公开研究）**

- [Trail of Bits: LUKS2 in confidential VMs (2025-10-30)](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)
- [HackTricks: LUKS2 header malleability / null cipher](https://cloud.hacktricks.wiki/en/pentesting-cloud/confidential-computing/luks2-header-malleability-null-cipher-abuse.html)

---

## 附录：本仓库里的演示

分离 header 的风险、触发条件和一次本地只读观察，见 [demos/luks2-detached-header/](demos/luks2-detached-header/README.zh.md)。
