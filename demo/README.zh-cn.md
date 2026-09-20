# CDF GPT demo (`cdf-gpt`)

无状态 Rust 工具，演示 [CDF v1 GPT 方案](../design/README.zh.md)。

### 状态机

```text
S0 裸盘/文件  --create[--force]-->  S1 已 seal 的 CDF  --open-->  S2 mapper
```

- **`create`**：唯一的 initialize；一次做完 GPT + seal。已是 CDF 时默认拒绝，需 `--force`。
- **`open`**：只校验并映射，**绝不** format；成功后打印拆除命令（不改盘）。
- **`info`**：只读查看。

`create` 两种容量来源：新文件用 `--size`；已有文件/块设备用自身容量（块设备禁止 `--size`）。

### 密钥

`--key` 为 `K_root`，经 **HKDF-SHA-256** 得到 `K_mac`（HMAC）与 `K_luks`（LUKS）。

### profile

共用一个 `create`，用 `--profile` 区分：

- `plain` / `crypt+integrity`：rw；create 后 open，再在容器里写数据即可。
- `verity` / `verity+crypt`：ro；**必须** `--from-image`（先定稿内容，再算哈希树并 seal）。

```bash
sudo ./target/release/cdf-gpt ./disk.img create \
  --profile crypt+integrity --size 256M --key 'demo-secret'

sudo ./target/release/cdf-gpt ./disk.img open --key 'demo-secret'
```
