# LUKS2 detached header (local observation)

Chinese write-up: [README.zh.md](README.zh.md).

The script shows a host replacing the on-disk metadata region, then contrasts
attached open (fails) with a format-time detached header (still reads the
payload). It does not reproduce a cipher-downgrade bypass.

```bash
sudo ./run-demo.sh
```
