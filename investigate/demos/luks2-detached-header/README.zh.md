# LUKS2 分离 header：风险、触发条件、本地观察

这个目录只做**防御性观察**：创建一张自己的文件型 LUKS2 盘，看清 header 是明文、校验和不是 MAC，以及 `--header` 实际改变了哪一步。第 9 步用「换上一张诚实的外国头」对照不可信 host 能改控制面；**不会**伪造 LUKS 校验和、不会注入空算法、不会给出机密性绕过步骤。公开研究见 [Trail of Bits, 2025-10-30](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)。

本机已装 `cryptsetup` 即可。需要能用 device-mapper（通常是 root）：

```bash
sudo ./run-demo.sh
```

产物写在 `work/`（已 gitignore）。`./run-demo.sh clean` 删除它们。

---

## 1. 分离 header 是什么

默认打开：

```
cryptsetup open /dev/DISK name
        └─ 现场从 DISK 字节 0 读 H_pri · J_pri
```

分离打开：

```
cryptsetup open --header /path/to/header.img /dev/DISK name
        └─ 从 header.img 读元数据，DISK 只当载荷
```

`header.img` 通常是 `cryptsetup luksHeaderBackup` 的结果，里面是整份 LUKS2 头（两份二进制头 + JSON + keyslot 区），**不含**用户数据。

形式化：

```
AttachedOpen  ::=  ParseHeader(Device) ; Activate(Device, Header)
DetachedOpen  ::=  ParseHeader(HeaderFile) ; Activate(Device, HeaderFile)

Device        ::=  Header · Keyslots · Payload     -- 默认
Device'       ::=  [任意前缀] · Payload            -- 分离后，Device 上的头可以被忽略
```

分离本身**不是漏洞**。CVM 项目用它，是为了让「校验」和「打开」用同一份已经在 guest 内存里的字节，避免 TOCTOU。

---

## 2. 真正的风险在哪（不是「用了 --header 就危险」）

LUKS2 规范写明：头上的 SHA-256 只防**意外损坏**，不防故意改写。默认数据面又是 AES-XTS（保长、不认证）。于是：

| 条件同时成立时 | 会发生什么 |
|----------------|------------|
| 存储 / host 能改这块设备 | 对手看得到、改得了 JSON（cipher、segment、token） |
| guest 信任现场盘上的 header | `cryptsetup open` 按这份 JSON 建 dm-crypt |
| 没有 MAC / 度量 / 白名单 | 改过的参数也会被接受 |
| 数据段被改成空算法（公开案例） | 口令仍能打开 keyslot，但新写入对 host 是明文 |

本机 cryptsetup 是 **2.7.0**。2.8.1 起会拒绝「非空口令 + keyslot 空算法」；**segment** 空算法在公开说明里仍可能被接受。本演示不去碰这些算法，只让你看见：**cipher 名字就在明文 JSON 里，校验和算法是 `sha256` 而不是 HMAC**。

### 分离 header 什么时候有用

1. header 副本是在**可信时刻**做的（例如 TEE 内 format 完立刻备份），并且
2. 之后每次打开都用**这份**副本（放 tmpfs / 内存），并且
3. 最好再给这份字节做 MAC，或把它度量进 attestation。

### 分离 header 什么时候没用，甚至更糟

| 触发条件 | 结果 |
|----------|------|
| 每次打开都对现场盘做 `luksHeaderBackup`，再 `--header` 打开 | 只是把不可信头拷进内存，**没有**建立信任 |
| header 文件和数据盘都在不可信存储上，且没有 MAC | 对手改 header 文件即可，和改盘上头同类 |
| header 文件丢失 / 与盘不匹配 | 数据无法打开（分离头的可用性代价） |
| 先 `luksDump` 校验盘上头，再**不带** `--header` 打开同一块盘 | 经典 TOCTOU：两次读之间 host 可以换头 |

所以：`--header` 是打开路径上的**必要手续**，不是完整对策。完整对策是「可信来源的头 + 分离打开 +（MAC 或度量）」。

---

## 3. 演示脚本会让你看见什么

`run-demo.sh` 按顺序做：

1. 建 64 MiB 文件盘，`luksFormat`（LUKS2）。
2. 写入固定明文标记 `CONFIDENTIAL-DEMO-PAYLOAD`，关闭映射。
3. 在**未解锁**的 raw 文件上搜这个标记：应该找不到（数据面在工作）。
4. 在 raw 文件前 64 KiB 里搜 `LUKS`、`aes-xts-plain64`：应该找得到（元数据是明文）。
5. `luksDump --dump-json-metadata`：直接打印 `segments["0"].encryption` 和 `config`。
6. 从二进制头读 `csum_alg`：应是 `sha256`（无密钥，改完 JSON 可以重算）。
7. **format 当时**备份一份 `header-at-format.img`，用它 `--header` 打开，读回标记。
8. 对比：若现在才从盘上再 backup 一次，得到的是「现场盘上的头」，在 CVM 里不能当可信锚。
9. 用白名单检查 **format 时** 的 JSON：只允许约定算法。
10. **不可信 host 路径（对照，不做算法降级）：**
    - 在磁盘副本上，用另一张诚实格式化的 LUKS2 头覆盖前 16 MiB（host 能写元数据区）。
    - **现场打开** + guest 原密钥：失败（cryptsetup 读到的是 host 的头）。
    - **分离打开** + format 时的头：仍读回标记（载荷在 16 MiB 之后，没被这次覆盖）。
    - 另写一份「host 呈上的」JSON 夹具（`encryption = UNTRUSTED-HOST-CONTROLLED`），白名单拒绝。这份夹具**不会**写回磁盘。

本目录**不会**提供把真实 LUKS 头改成空算法、重算校验和再打开的步骤。那是已公开的机密性绕过，不是防御演示。公开叙述见 [Trail of Bits](https://blog.trailofbits.com/2025/10/30/vulnerabilities-in-luks2-disk-encryption-for-confidential-vms/)。

看完应能建立的模型：

```
对 cryptsetup 而言：密钥是打开时直接喂进去的 key-file / 口令。
对 agent 而言：KBS URI 只决定「去哪取」这串字节，LUKS 并不认识 URI。
对 CVM host 而言：header JSON 是可以读、在未加 MAC 时也可以换的控制面。
```

---

## 4. 不可信 host 的攻击路径（概念，不是复现菜谱）

```
                    不可信 host / 存储
                            |
                            |  能读写 Device[0 .. payload_offset)
                            |  （明文 H_pri · J_pri · keyslots）
                            v
+------------------+   现场 open    +------------------+
| Device（被换头）  | -------------> | guest cryptsetup |
+------------------+                +------------------+
        |                                    |
        | 分离 + 可信 H                      | 若相信现场头
        v                                    v
  仍按 format 时的 cipher/keyslot      按 host 此刻呈上的 JSON 建映射
  打开原来的 Payload                   （控制面已被 host 替换）
```

同时成立时，公开研究里的机密性事故才会发生：host 改 JSON、guest **现场**打开、没有 MAC/度量/白名单。本演示用「换上一张诚实的外国头」证明**控制面在 host 手里**；用白名单拒绝夹具证明 **guest 必须 fail-closed**。完整对策仍是：可信来源的头 + `--header` + MAC 或度量。
