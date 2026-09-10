# CDF v1 — 机密磁盘格式

**状态。** 草案。  
**日期。** 2026-09-10。  
**调研。** [investigate/README.zh.md](../investigate/README.zh.md)（[English](../investigate/README.md)）。  
**规范正文（以英文为准）。** [README.md](README.md)。

这是盘上契约：同一份镜像既要能被 CoCo guest（CDH / Trustee）打开，也要能被不跑 Kata 的 CVM 打开。

---

## 1. 结论：没有现成标准能用

仓库目标是 R1–R5（[调研 §1](../investigate/README.zh.md)）：可选加密 / 完整性、CoCo **且**无侧车的 CVM、自描述、Linux dm-crypt 开箱，以及**被认证的**打开契约。

调研过的格式**没有**一条同时满足这些。用调研的分类法（[§5](../investigate/README.zh.md#5-按磁盘格式分类)）来说，CDF v1 是 **M 类 + E 类**：分区表带来的可发现性，叠加一块被认证的区域。现有项目没有一个同时做到这两点。CDF 包住的数据面对象全部是已有标准（见 §2）。

| 候选 | 为什么不能当本格式 |
|------|-------------------|
| **LUKS2** | 自描述加密容器（[规范](https://gitlab.com/cryptsetup/LUKS2-docs)），但头不能抵抗 host。不过 R5，也不过 ToB 的 CVM 威胁模型。 |
| **LUKS2 + `kbs` token**（[libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs)） | 盘上有 KBS 定位符。头仍未认证；要 cryptsetup 插件；没有 verity，也没有 GPT 发现。 |
| **systemd-homed**（[HOME_DIRECTORY](https://systemd.io/HOME_DIRECTORY/)） | 最好的可移植 `GPT · LUKS2 · token` 镜像。元数据是桌面用户记录，不是 attestation + KBS。头仍可被 host 改。 |
| **Clevis / systemd-cryptenroll** | 盘上写「密钥怎么取」，不是 CVM 卷镜像。 |
| **cryptpilot** | 产品化的 CVM + KBS + 可选 integrity / verity。密钥 URI 和 verity root 在 **guest TOML / initrd**，磁盘不自描述（R2、R3）。 |
| **CAA CSI + Kata #13638** | 能跑的 CoCo 路径。打开配置在 **宿主机 `mountInfo.json` 或 Trustee manifest**，不在镜像上。只有盘的 CVM 过不了 R2/R3。 |
| **CDH #1648 / #1663**、**tdx-init** | 信封是对的（MAC + 分离打开 + fail-closed）。布局绑实现，不是可移植 GPT 镜像，也没有 verity 分区和 type UUID。 |
| **Constellation / Contrast / dstack** | 分离头 + JSON 白名单（或 MAC）。密钥在集群里，没有可移植规范。 |
| **Azure / GCP / IBM SE / qcow2+LUKS** | 平台或虚拟镜像契约，不是随处可挂的 raw 卷。 |
| **VeraCrypt、BitLocker、TCG Opal、ZFS、fscrypt、ocicrypt** | 数据面不对「`cryptsetup` + CDH `SecureMount`」（R4）。 |

**复用、不替换：** LUKS2、`dm-integrity`、`dm-verity`、GPT。  
**新造：** 一块用 GPT 就能发现的 meta 分区，打开参数用 host 没有的根密钥做 **MAC**。里面没有任何秘密，所以只认证、不加密（§7.1）。

---

## 2. 来源（借了什么）

| 部件 | 来源 | 在 CDF 里的角色 |
|------|------|-----------------|
| 按 type UUID + label 发现 GPT 分区 | [UEFI GPT](https://uefi.org/specs/UEFI/2.10/05_GUID_Partition_Table_Format.html)；模式来自 [systemd-homed](https://systemd.io/HOME_DIRECTORY/) | 不靠 CoCo 也能找到 `P_meta` / `P_verity` / `P_data` |
| LUKS2 容器 + token | [LUKS2 盘上格式](https://gitlab.com/cryptsetup/LUKS2-docs) | 机密可写载荷 |
| 分离 header，`open --header` | [cryptsetup](https://gitlab.com/cryptsetup/cryptsetup)；ToB 对 CVM 的建议 | 关 TOCTOU；数据分区只放载荷 |
| `dm-integrity`（`hmac-sha256`） | [内核 dm-integrity](https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html)；cryptsetup `--integrity` | 可变数据的篡改证据 |
| `dm-verity` + 512 B superblock | [内核 dm-verity](https://docs.kernel.org/admin-guide/device-mapper/verity.html)；[cryptsetup wiki](https://gitlab.com/cryptsetup/cryptsetup/-/wikis/DMVerity) | 不可变只读树 |
| 对明文头做 MAC | [Trail of Bits (2025-10-30)](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)；[tdx-init v0.2.0](https://github.com/flashbots/tdx-init)；[CDH #1648](https://github.com/confidential-containers/guest-components/pull/1648) | 认证打开契约。两个先例都是对明文头做 MAC，而不是加密它 |
| fail-closed 重开；解锁失败不 format | [CDH #1648](https://github.com/confidential-containers/guest-components/pull/1648) | 已有卷不能变成 from-scratch |
| 持久 LUKS 的带日志 integrity | [CDH #1663](https://github.com/confidential-containers/guest-components/pull/1663) | 推荐的 RW 完整性配置 |
| 密钥定位符**不放**在盘上 | [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs)（优先用 SMBIOS 的 `initdata.toml` 而不是盘上字段）；[Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638)（manifest URI 由被度量的 policy 授权） | §6.2：镜像带身份，部署带定位符 |
| HKDF 拆根密钥 | 常规 KDF（[HKDF / RFC 5869](https://www.rfc-editor.org/rfc/rfc5869)） | `K_mac` 不能靠解析不可信 LUKS 头得来 |
| 元数据代号计数器 | 常规的元数据版本化做法 | 拒绝在同一 `K_mac` 下被贴回来的旧 Contract（§8.4） |

---

## 3. 威胁模型与安全目标

**对手。** Host / 存储能读写镜像的每一个字节。它拿不到 `K_root`（只在证明之后释放，或由 CVM 所有者从可信路径提供）。

**目标。**

| ID | 目标 | CDF 怎么做 |
|----|------|------------|
| G1 | 可选机密 / 完整性 | §5 的 profile；记在被 MAC 覆盖的 Contract 里 |
| G2 | CoCo 和 CVM | GPT type UUID + 盘上定位符；不要求只存在于 Kata 的侧车 |
| G3 | 自描述 | 只有一份明文 Contract：可直接读来发现，但只有 MAC 通过后才具权威性 |
| G4 | 开箱数据面 | 只用 `cryptsetup` + device-mapper |
| G5 | 换头不能成功打开 | 用 `K_mac` 对整份契约做 HMAC；用校验通过的头做分离打开 |

**非目标。**

- **整设备的新鲜度（回滚 / 重放）。** host 可以把整个镜像换成一份更旧但自洽的副本：每个 `dm-integrity` 标签都能过，Contract 的 MAC 也能过，因为两者都是同一批密钥产生的。CDF **不检测**这件事，并且刻意不为它规定任何机制。要做这件事必须依赖盘外的可变状态（一个记住每卷最新代号的有状态密钥服务，或一个 guest 可见的单调计数器），而上游 Trustee 并不提供；[Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638) 出于同样理由把同一场景列为非目标（原话：「neither the disk nor Trustee records which authenticated state is newest」）。确实需要的部署必须自己加一个外部新鲜度锚点，并把它当作本格式之上的一层。
- 会话内单个扇区的重放。`dm-integrity` 的标签绑扇区号、不绑写入版本，所以同一密钥下把旧的 `(sector, tag)` 对重放回去检测不到。
- RW-many 互斥；隐瞒「这是 CDF 盘」；可否认性；信任磁盘固件（OPAL）。

换头导致 `open` **失败**，一般是可用性，旧 AES-XTS 密文仍机密。CDF 要禁止的是 `open` **成功**但数据面是 host 选的（新写入，或 mount 失败后 format，变得对 host 可见）。已有卷和 from-scratch 都适用。

---

## 4. 磁盘格式

### 4.1 镜像

```
Image ::= GPT( P_meta [, P_verity], P_data )

P_meta.type_uuid    = 4ca88ee1-60f4-4b6b-899e-dae026ced8e8   -- "CDF meta"
P_verity.type_uuid  = 9620d886-ff76-4ccd-9b09-3a938a18dc0d   -- "CDF verity"
P_data.type_uuid    = e4ff66b9-eebf-4675-b8a0-836ca79919ce   -- "CDF data"

P_meta.label        = "cdf-meta"
P_verity.label      = "cdf-verity"          -- 不用 verity 则没有
P_data.label        = "cdf-data"
```

`P_meta` 宜至少 32 MiB（整份 LUKS2 头备份常见 16 MiB）。  
不用 verity 就省略 `P_verity`。  
`P_data` 吃掉剩余可用容量。

```
+--------+-------------+----------------------+---------------------------+
| GPT    | P_meta      | [P_verity]           | P_data                    |
|        | CDF 信封    | verity SB + HashTree | 载荷（见 profile）        |
+--------+-------------+----------------------+---------------------------+
```

### 4.2 Meta 分区

Superblock 里多字节整数均为 **小端**。未用字节填 0。

元数据分区是**明文且被认证的**。打开契约只有一份，不需要任何密钥就能读，但只有 MAC 通过之后才**可用**。为什么这里只认证不加密，见 §7.1。

```
P_meta ::= Superblock_4096 · Contract · Pad

Superblock ::=
    magic[8]           = "CDF1" || 0x00 0x00 0x00 0x00
    version_major      : u16        -- 1
    version_minor      : u16        -- 0
    volume_uuid        : u8[16]     -- RFC 4122 二进制 UUID
    flags              : u32        -- 见下
    contract_offset    : u32        -- 相对 P_meta 起点；通常 4096
    contract_size      : u32        -- Contract 的精确字节数
    meta_generation    : u64        -- 从 1 起；每次改写 Contract 递增
    mac                : u8[32]     -- HMAC-SHA-256；计算时该字段填零
    reserved           : 零填到 4096
```

`flags`（按位或）：

| 位 | 名称 | 含义 |
|----|------|------|
| 0 | `AUTHENTICATED` | 存在 `mac`，且必须先校验通过才能使用 Contract 的任何字段 |
| 1 | `HAS_VERITY` | 存在 `P_verity` |
| 2 | `INITIALIZED` | 首次 format 成功后置位 |

这个标志位是**关于本镜像的提示，不是安全判断**：host 可以把它清掉。一个卷是否**必须**被认证，来自部署侧（被度量的 policy、CoCo manifest、CVM 所有者配置）。当打开方预期的是机密卷、而镜像给出 `flags.AUTHENTICATED == 0` 时，必须拒绝，处理方式与 MAC 校验失败完全相同。

**MAC**

```
mac = HMAC-SHA-256( K_mac,
                    Superblock[ mac := 0^32 ]              -- 全部 4096 字节
                    || Contract[ 0 .. contract_size ) )
```

把整个 superblock 纳入覆盖范围，就认证了 `volume_uuid`、`flags`、`meta_generation` 以及偏移/长度这一对，于是 host 既不能挪动或截断 Contract，也不能重新解释「哪些字节被覆盖」。`contract_size` 本身被认证，因此两段输入之间不存在拼接歧义。比较必须是常数时间的。

`meta_generation` 也在 MAC 输入里，所以为某一代写的 Contract 无法被贴到更晚的一代上。这一条只覆盖元数据，**不是**整设备的新鲜度保证——后者是明确的非目标（§3）。

**Contract**（明文 JSON，长度恰好 `contract_size` 字节，不要求以 NUL 结尾）

```json
{
  "format": "cdf/v1",
  "volume_uuid": "<与 superblock 相同>",
  "meta_generation": 1,
  "profile": "plain" | "crypt" | "crypt+integrity" | "verity" | "verity+crypt",
  "data": {
    "luks2_header_b64": "<分离的 LUKS2 头，若 profile 使用 LUKS>",
    "luks2_subsystem": "cdf-v1",
    "integrity": "none" | "hmac-sha256",
    "cipher": "aes-xts-plain64"
  },
  "verity": {
    "root_hash_hex": "...",
    "algorithm": "sha256",
    "data_block_size": 4096,
    "hash_block_size": 4096,
    "salt_hex": "..."
  }
}
```

`profile` 含 `verity` 时必须有 `verity`。含 `crypt` 时必须有 `luks2_header_b64`。MAC 通过后，`volume_uuid` 和 `meta_generation` 必须与 superblock 一致（这两个字段刻意重复一遍，好让只读 JSON 的解析器不会和二进制头脱节）。

**Contract 里不得包含任何秘密。** 这是一条规范性约束，不是一句观察：正因为如此，用 MAC 才够。将来某个版本若确实需要携带秘密字段，必须另定义一个加密扩展，而不是塞进这里。

**盘上没有密钥定位符。** 这里刻意没有 `kbs_uri` 字段，理由见 §6.2。

### 4.3 数据和 verity 分区

```
-- profile plain
P_data ::= Filesystem

-- profile crypt | crypt+integrity
P_data ::= Payload                        -- 没有现场 LUKS 头
           cryptsetup open --type luks2 --header H_ram P_data
           H_ram = decode(Contract.data.luks2_header_b64)

-- profile verity | verity+crypt
P_verity ::= VeritySB_512 · PadTo(hash_block) · HashTree
P_data   ::= DataBlocks | LUKS2_Payload
             root hash 取自 Contract.verity，绝不单独信任 P_verity
```

`P_data` 上的 LUKS2 用**分离头**创建（`cryptsetup luksFormat --header …`）。盘上 `P_data` 起点就是密文（或 integrity 日志 + 密文）。Host 在 `P_data` 开头写一块「看起来像 LUKS 头」的字节，打开路径**必须忽略**。

`profile` 含 `crypt` 时的默认 LUKS2 参数：

| 参数 | 值 |
|------|-----|
| Cipher | `aes-xts-plain64` |
| 密钥长度 | 512-bit XTS |
| Integrity | 仅 `crypt+integrity` 时用 `hmac-sha256` |
| 扇区 | 4096 |
| `subsystem` | `cdf-v1` |
| `label` | 文本形式的 volume UUID |

---

## 5. Profile

| `profile` | 机密性 | 完整性 | 典型用途 |
|-----------|--------|--------|----------|
| `plain` | 无 | 无 | 非秘密划痕盘；仍可用 GPT 发现 |
| `crypt` | LUKS2 | 无 | 可写秘密数据，无逐扇区标签 |
| `crypt+integrity` | LUKS2 | `dm-integrity` | 默认机密 RW 卷 |
| `verity` | 无 | 只读树 | 不可变公开镜像 |
| `verity+crypt` | 先 LUKS2 再 verity | 两者 | 不可变秘密镜像 |

除 `plain` 之外的每个 profile 都必须置位 `flags.AUTHENTICATED`：`verity` 虽然不含秘密，但同样需要一个被认证的 root hash。`plain` 没有密钥，因此没有 MAC，打开方必须把它当成普通的无保护分区。

对每个被认证的 profile，打开方必须从**MAC 校验通过**的 Contract 里取 `profile`。校验之前读到的同一批字节只能用于展示。

---

## 6. 密钥

### 6.1 层次

```
K_root     <-- 证明之后的 KBS，或 CVM 所有者的 key file
              绑 volume_uuid；禁止多卷共用一把

K_mac      = HKDF-SHA-256(K_root, salt=volume_uuid, info="cdf-v1/meta-mac")
K_luks     = HKDF-SHA-256(K_root, salt=volume_uuid, info="cdf-v1/luks")

HMAC       用 K_mac
LUKS2      口令 / key-file 是 K_luks   -- 不是裸 K_root，也不是从盘上头解出来的 key
```

`K_mac` **绝不能**从「解析盘上 LUKS 头得到的 volume key」派生。那个顺序等于先信 host 控制的头，再用它来认证自己。

`K_luks` 是一把均匀随机的 256 位密钥，不是人类口令。由此有两个推论：明文公开 LUKS2 头不会让 keyslot 面临离线猜测攻击；keyslot 的 KDF 加固也不承担安全职责（它保护的是低熵口令，而 CDF 不使用低熵口令）。

### 6.2 密钥获取走带外

**镜像不说明自己的密钥从哪来。** 它携带卷的*身份*（`volume_uuid`），定位符由部署侧携带。打开方以 `volume_uuid` 为索引，从可信通道取得 `K_root`：

| 通道 | 例子 |
|------|------|
| 被度量的 guest 配置 | CoCo Init-Data（`cdh.toml`）、SMBIOS OEM 字符串里的 `initdata.toml`（[`kbs` token 插件](https://github.com/MatiasVara/libcryptsetup-token-kbs)本身就优先用这个而不是盘上字段） |
| 被度量的编排策略 | 由被度量的 Agent policy 授权的 Trustee manifest 引用（[Kata #13638](https://github.com/kata-containers/kata-containers/issues/13638)） |
| CoCo 存储链路 | StorageClass / CSI 路径里的 `kbs-key-id` |
| CVM 所有者 | 所有者在启动时给的 key file 或 KMS 引用 |

定位符不该放在镜像上，有三个理由：

1. **它来不及被认证。** 定位符是在拿到任何密钥**之前**就需要的，所以它必然在尚未验证的状态下被消费。把它放在攻击者可写的介质上，打开方无论如何都得拿一份可信清单去核对——而那一刻，可信清单才是真正的来源，盘上那份只多贡献了一个「不一致怎么办」的分支。
2. **解释字节不需要它。** 算法、布局、完整性配置、verity 根，这些是镜像的属性；密钥由哪个服务保管，是环境的属性。
3. **它把镜像绑死在一个部署上。** 否则一旦密钥服务换地址，或镜像要在环境间搬动，就得改写被认证的元数据、为一次「路由变更」白烧一代。

交给 volume 的永远是 **密钥字节**（`K_luks`），不是 URI。KBS 宜只在请求匹配 `(attestation, volume_uuid)` 时释放 `K_root`。`K_root` 是每卷一个的长期秘密，本格式**不**在打开时轮换它；轮换是管理操作（§8.4）。

打开方输入的可信程度：

| 通道 | 提供什么 | 信任 |
|------|----------|------|
| MAC 校验前的 Superblock + Contract | 用于查密钥的 `volume_uuid`；其余字段仅供展示 | **不可信** |
| 被度量的 CoCo / CVM 配置 | 定位符，或直接给 `K_root` | 被度量则可信 |
| KBS | `K_root` | 证明之后可信 |
| MAC 校验后的 Superblock + Contract | 整份打开契约 | 可信 |

校验前读 `volume_uuid` 是安全的：host 若把它换掉，打开方就会去要另一把密钥、派生出错的 `K_mac`，随后 MAC 校验失败——因为 `volume_uuid` 就在 MAC 输入里。失败模式是「拒绝打开」，不是「错误地打开」。已经知道预期 `volume_uuid` 的部署，宜先比对再往下走。

---

## 7. 算法

| 用途 | 算法 | 说明 |
|------|------|------|
| 元数据认证 | **HMAC-SHA-256** | 32 字节 tag，放在 superblock 里；无 nonce；只在改写 Contract 时计算 |
| KDF | HKDF-SHA-256 | RFC 5869 |
| LUKS2 载荷 | `aes-xts-plain64` | 白名单；拒绝包括空算法在内的其他 cipher |
| RW 完整性 | 经 LUKS2 `--integrity` 的 `hmac-sha256` | 开日志 |
| RO 完整性 | `sha256` dm-verity | root hash 在 Contract 里 |

符合规范的打开方：即使 MAC 已通过，也必须拒绝 `Contract.data.cipher` 不在 `{aes-xts-plain64}` 内，以及 LUKS2 JSON 里 segment encryption 与该值不一致的头。

### 7.1 为什么用 MAC，而不用 AEAD

威胁是 host **改写**打开契约，不是 host **读到**它。Contract 里每个字段要么本来就是公开的，要么在别处已经公开：

| 字段 | 是秘密吗 | 为什么不是 |
|------|----------|-----------|
| `profile`、`cipher`、`integrity` | 不是 | 从分区大小、以及 host 本就能看见的 `dm-integrity` / verity 几何就能推出来 |
| `verity.root_hash_hex`、`salt_hex` | 不是 | verity 根哈希本身就是一个公开承诺，别的系统还会给它签名并公开发布 |
| `luks2_header_b64` | 不是 | 任何普通 LUKS 设备上的 LUKS2 头都是明文存放的。它的 keyslot 里装的是被 `K_luks` 包裹的 volume key，而 `K_luks` 是满熵的 256 位密钥（§6.1），所以泄露它并不带来离线攻击 |
| `volume_uuid`、`meta_generation` | 不是 | 身份和版本，本来就在明文 superblock 里 |

既然如此，加密什么都没买到，还要付代价：

- **不需要管 nonce。** AES-GCM 要求同一密钥下每次密封的 nonce 唯一，而 nonce 重用丢掉的是认证密钥，不只是明文。HMAC 没有这个失效模式。契约会在每次换密钥、改 profile 时重新密封，可能由不同工具、甚至在崩溃之后进行——恰好是 nonce 纪律最容易出问题的场景。
- **一个对象而不是两个。** 契约若被加密，格式就必须再放一份明文结构，好让「还没有任何密钥时」也能认出这个卷，并且额外规定两份必须一致。改成对明文契约做认证，这份重复和这条一致性规则一起消失。
- **可恢复、可排查。** `blkid` 式的工具、排查挂载失败的运维、以及取证读取者，都能在不持有密钥的情况下看清这个卷声称自己是什么，同时仍然改不动它。
- **与先例一致。** tdx-init 和 CDH #1648 都是对明文 LUKS2 头做 MAC，谁也没有加密它。

如果契约确实需要携带秘密——被包裹的密钥、某个 token、不想让 host 看到的策略——那 AEAD 才是对的选择。它不需要，而且 §4.2 已经把这一点写成规范性约束，好让这个前提不会悄悄腐烂。

MAC 方案必须做对的地方，也正是「只用 MAC」会翻车的地方：必须同时覆盖二进制 superblock 和 JSON（否则偏移、`flags`、`volume_uuid` 都还是可塑的）；被覆盖的长度本身必须被认证（否则两段输入的边界有歧义）；tag 比较必须是常数时间。§4.2 三条都规定了。

---

## 8. 手续

`Zero(P)` 表示分区 `P` 每个字节都是 0x00。

### 8.1 识别

1. 解析 GPT。必须有 type 为 `4ca88ee1-60f4-4b6b-899e-dae026ced8e8` 的 `P_meta`。
2. 读 Superblock。`magic` 必须是 `CDF1\0\0\0\0`，`version_major` 必须是 1。
3. 读 `volume_uuid`。这一步读到的其他一切都不可信。

### 8.2 首次 format（init）

仅当**同时**满足：`Zero(P_meta)`、`Zero(P_data)`、`[P_verity]` 不存在或全零；**并且**策略允许 init（CoCo：被度量的 `readWrite` + empty source；CVM：所有者动作）。

1. 分配 `volume_uuid`，`meta_generation := 1`。
2. 取得 `K_root`（§6.2）。派生 `K_mac`、`K_luks`。
3. 若 profile 用 LUKS：在 tmpfs 里 `luksFormat` 分离头；口令 `K_luks`；参数见 §4.3。不要把 LUKS 头写到 `P_data` 上。
4. 若 profile 用 verity：在 `P_verity` 上建哈希树；root hash 写入 Contract。
5. 序列化 Contract，填好 Superblock（`INITIALIZED|AUTHENTICATED|…`、`contract_size`、`mac := 0`），对两者计算 MAC 并写入 `mac`。
6. 先写 Contract，最后写 Superblock，这样崩溃留下的镜像会识别失败，而不是「对着半份契约校验通过」。
7. 按 §8.3 打开，把 mapper 交给文件系统 / 工作负载。

### 8.3 打开（重开）

下面这个顺序的全部意义在于：**在 MAC 校验通过之前，镜像里的任何一个字节都不影响 device-mapper 的决策。**

1. **识别**（§8.1）。从 Superblock 取 `volume_uuid`。
2. **确定预期**，依据来自部署侧而不是镜像：这个卷是否必须被认证？是只读还是读写？如果部署预期它被认证、而 `flags.AUTHENTICATED == 0`，**拒绝**。
3. Superblock 全零或 magic 未知：
   - 若满足 §8.2 的 init 条件 → format；
   - 否则**拒绝**。非全零或写到一半的镜像绝不 format。
4. **取密钥。** 从可信通道解析定位符（§6.2），做证明，拿到 `K_root`。派生 `K_mac` 和 `K_luks`。
5. **校验。** 重算 `HMAC-SHA-256(K_mac, Superblock[mac := 0] || Contract[0 .. contract_size))`，用常数时间比较。
   - **不一致：拒绝。** 不 format，不回退成 `plain`，不在没有头的情况下打开 `P_data`，也不换一种解释重试。
6. **交叉核对。** `Contract.volume_uuid`、`Contract.meta_generation` 必须与 Superblock 一致；`Contract.profile` 必须在部署允许的集合内；`Contract.data.cipher` 必须是 `aes-xts-plain64`。
7. **激活**，顺序由 `profile` 决定：
   - 有 verity 就先激活 verity：`dm-verity` 用 Contract 里的 root hash，绝不用 `P_verity` 自己的 superblock；
   - 然后是 LUKS：把 `luks2_header_b64` 写到 tmpfs 文件 `H_ram`，白名单检查它的 JSON，`cryptsetup open --header H_ram`。
8. **挂载**，权限用第 2 步定下的那个，然后返回 mapper。

以 `crypt+integrity` 为例，假设 `K_root` 已经在 guest 内存里：

```sh
# 1-3. 识别并读取，此时仍然不可信
sb=$(dd if=/dev/sda1 bs=4096 count=1 status=none | xxd -p)      # P_meta 的 superblock
uuid=$(cdf-tool show-uuid /dev/sda1)                            # 校验前唯一被使用的字段

# 4. 以身份为索引，从可信通道取密钥
K_root=$(cdf-tool get-key --uuid "$uuid")                       # 证明在这一步发生
K_mac=$( hkdf-sha256 "$K_root" --salt "$uuid" --info cdf-v1/meta-mac)
K_luks=$(hkdf-sha256 "$K_root" --salt "$uuid" --info cdf-v1/luks)

# 5. 先校验再做别的；非零退出意味着停下，不是回退
cdf-tool verify /dev/sda1 --key "$K_mac" || exit 1

# 6-7. 到这一步契约才可用
cdf-tool extract-header /dev/sda1 > /run/cdf/H_ram             # tmpfs，guest 内存
cdf-tool check-header /run/cdf/H_ram --allow-cipher aes-xts-plain64 || exit 1
printf '%s' "$K_luks" | cryptsetup open --type luks2 \
    --header /run/cdf/H_ram --key-file - /dev/sda2 cdf-data

# 8.
mount -o ro /dev/mapper/cdf-data /mnt/data                     # 只有第 2 步说可写时才 rw
```

注意这里**没有**什么：没有不带 `--header` 直接对 `/dev/sda2` 做 `cryptsetup open`，整条路径上没有 `luksFormat`，也没有任何一个分支把「校验失败」当成「这一定是块新盘」。

### 8.4 改写 Contract

任何经授权的改写（换密钥、改 profile、换 verity 根）都必须：增加 `meta_generation`、重算 MAC，并且和 §8.2 一样先写 Contract 再写 Superblock。这样早一代的 Contract 对当前 Superblock 校验就会失败，host 无法通过恢复旧字节来撤销一次已完成的换密钥或改 profile。

改写不在打开路径上：打开卷永远不改 `meta_generation`，因此密钥服务侧不需要任何可写状态。

---

## 9. CoCo 和裸 CVM 怎么用同一份镜像

```
                         +------------------+
                         | CDF 镜像 (GPT)   |
                         +------------------+
                                  |
              用 type UUID 发现 P_meta — 不需要插件
                                  |
              +-------------------+-------------------+
              |                                       |
     CoCo：被度量的策略                         CVM：所有者 / cloud-init
     提供密钥定位符                             提供定位符或 K_root
              |                                       |
              +-------------------+-------------------+
                                  |
                    用镜像里的 volume_uuid 去查
                                  |
                         KBS（可选） --> K_root
                                  |
                         校验 MAC --> Contract 可用
                                  |
                         cryptsetup --header H_ram
```

定位符来自上面两条路径中存在的那一条；镜像永远不提供它（§6.2）。CoCo 仍可以把 `kbs-key-id` 放在 CSI 元数据里——那是不可信传输，之所以可以接受，是因为随后的 MAC 会证明这把密钥是对的：错误的或攻击者挑的密钥只会导致校验失败、拒绝打开。手里只有盘的 CVM 需要所有者给出定位符或 `K_root`；单靠一张盘是不够的，这是刻意的。

---

## 10. MAC 单独消不掉的剩余攻击

| 攻击 | 结果 | 额外控制 |
|------|------|----------|
| 没有 `K_mac` 就改 Contract 或 Superblock 的任何字段 | MAC 失败；拒绝打开 | §8.3 fail-closed |
| 清掉 `flags.AUTHENTICATED`，把卷伪装成 `plain` | 被拒绝 | 预期形态来自部署侧而不是镜像（§4.2、§8.3 第 2 步） |
| 改 `P_data` 开头的现场字节 | 被忽略（`--header H_ram`） | 分离载荷 |
| 只恢复这份 `K_mac` 下**旧的** Contract | 被拒绝 | `meta_generation` 在 MAC 输入里（§8.4） |
| 替换 `volume_uuid` 以改变密钥查找 | MAC 失败；拒绝打开 | `volume_uuid` 在 MAC 输入里（§6.2） |
| 换成另一张用**同一把** `K_root` 封过的 CDF 盘 | MAC 可能成功 | 每个 `volume_uuid` 一把 `K_root`；部署侧核对预期 UUID |
| 把**整个镜像**换成更旧的自洽副本 | 会被接受；标签和 MAC 都能过 | **不在范围内**（§3）。需要外部新鲜度锚点 |
| 读取 Contract | 按设计就是能读 | 里面没有秘密（§7.1）；载荷仍然是加密的 |
| 翻转密文扇区 | 仅当 `crypt+integrity` / verity 时能发现 | 选对 profile |

---

## 11. 符合性检查单

实现算作 CDF v1 符合，若它：

1. 使用 §4.1 的 GPT type UUID。
2. 用 HKDF 得到的 `K_mac` 做 HMAC-SHA-256 认证 Contract，覆盖范围是「`mac` 置零的 Superblock」后接「恰好 `contract_size` 字节的 Contract」，并用常数时间比较。
3. 在比较成功之前，除了用于查密钥的 `volume_uuid`，不使用 Contract 的任何字段。
4. 只用校验通过的 Contract 里的头做 `--header` 打开 LUKS2。
5. 即使 MAC 通过，也拒绝空算法 / 非白名单 cipher。
6. 除 §8.2 外永不 format，且绝不因为校验失败而 format。
7. 「必须被认证」和「只读还是读写」取自部署侧，而不是镜像。
8. 盘上不存密钥定位符（§6.2）。
9. Contract 里不放秘密（§4.2）。
10. 每次改写 Contract 都递增 `meta_generation`，且从不递减。

---

## 12. 参考

- [LUKS2 On-Disk Format Specification](https://gitlab.com/cryptsetup/LUKS2-docs)
- [dm-integrity](https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html)
- [dm-verity](https://docs.kernel.org/admin-guide/device-mapper/verity.html)
- [UEFI GPT](https://uefi.org/specs/UEFI/2.10/05_GUID_Partition_Table_Format.html)
- [RFC 5869 HKDF](https://www.rfc-editor.org/rfc/rfc5869)
- [Trail of Bits: LUKS2 in confidential VMs](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)
- [flashbots/tdx-init](https://github.com/flashbots/tdx-init)
- [guest-components #1648](https://github.com/confidential-containers/guest-components/pull/1648)、[#1663](https://github.com/confidential-containers/guest-components/pull/1663)
- [systemd Home Directories](https://systemd.io/HOME_DIRECTORY/)
- [libcryptsetup-token-kbs](https://github.com/MatiasVara/libcryptsetup-token-kbs)
- 本仓库调研：[investigate/README.zh.md](../investigate/README.zh.md)
