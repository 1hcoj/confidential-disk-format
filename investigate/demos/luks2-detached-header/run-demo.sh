#!/usr/bin/env bash
# Defensive observation only: create a local LUKS2 file image, show that the
# header is cleartext + unkeyed SHA-256, open with a format-time detached
# header, then contrast an untrusted-host metadata swap on a copy.
# Does not forge LUKS checksums or downgrade ciphers.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
WORK="${ROOT}/work"
DISK="${WORK}/disk.img"
KEY="${WORK}/keyfile"
HDR_TRUSTED="${WORK}/header-at-format.img"
HDR_LIVE="${WORK}/header-from-live-device.img"
JSON="${WORK}/header.json"
MAPPER="cdf-luks2-demo"
PAYLOAD='CONFIDENTIAL-DEMO-PAYLOAD'

log() { printf '\n==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

cleanup_mapper() {
  if [[ -e "/dev/mapper/${MAPPER}" ]]; then
    cryptsetup close "${MAPPER}" || true
  fi
}

clean() {
  cleanup_mapper
  rm -rf "${WORK}"
  echo "removed ${WORK}"
}

if [[ "${1:-}" == "clean" ]]; then
  clean
  exit 0
fi

need_cmd cryptsetup
need_cmd python3
need_cmd grep

if [[ "${EUID}" -ne 0 ]]; then
  echo "this demo needs device-mapper access; re-run as root (sudo $0)" >&2
  exit 1
fi

clean >/dev/null
mkdir -p "${WORK}"
printf 'demo-passphrase-not-a-secret\n' >"${KEY}"
chmod 600 "${KEY}"

log "1. Create a 64 MiB LUKS2 file image"
dd if=/dev/zero of="${DISK}" bs=1M count=64 status=none
cryptsetup luksFormat --type luks2 --batch-mode \
  --pbkdf pbkdf2 --pbkdf-force-iterations 1000 \
  --key-file "${KEY}" "${DISK}"
cryptsetup luksHeaderBackup --header-backup-file "${HDR_TRUSTED}" "${DISK}"
note "format-time header copy: ${HDR_TRUSTED}"

log "2. Write a recognizable plaintext marker, then close"
cryptsetup open --key-file "${KEY}" "${DISK}" "${MAPPER}"
printf '%s\n' "${PAYLOAD}" >"/dev/mapper/${MAPPER}"
sync
cryptsetup close "${MAPPER}"

log "3. Raw image must NOT contain the marker (data plane encrypts)"
if grep -a -F -q "${PAYLOAD}" "${DISK}"; then
  echo "unexpected: marker found in ciphertext" >&2
  exit 1
fi
note "grep on ${DISK}: marker absent (good)"

log "4. Raw image DOES contain cleartext LUKS metadata"
python3 - <<'PY' "${DISK}"
import sys
path = sys.argv[1]
data = open(path, "rb").read(65536)
needles = [b"LUKS", b"aes-xts-plain64", b"sha256"]
for n in needles:
    print(f"    found {n!r} in first 64 KiB: {n in data}")
# luks2_hdr_disk.csum_alg starts at offset 72
csum_alg = data[72:104].split(b"\x00", 1)[0].decode("ascii", "replace")
print(f"    binary header csum_alg = {csum_alg!r}  (unkeyed hash, not HMAC)")
PY

log "5. Dump JSON metadata without unlocking the payload"
cryptsetup luksDump --dump-json-metadata "${DISK}" >"${JSON}"
python3 - <<'PY' "${JSON}"
import json, sys
j = json.load(open(sys.argv[1]))
seg = j["segments"]["0"]
print("    segment[0].encryption =", seg.get("encryption"))
print("    segment[0].offset     =", seg.get("offset"))
print("    segment[0].integrity  =", seg.get("integrity", "<none>"))
print("    keyslots              =", sorted(j["keyslots"]))
print("    tokens                =", j.get("tokens") or "<empty>")
print("    header is readable without the volume key; only key material is wrapped")
PY

log "6. Detached open using the format-time header (trusted copy in this demo)"
cryptsetup open --header "${HDR_TRUSTED}" --key-file "${KEY}" "${DISK}" "${MAPPER}"
got="$(head -n 1 "/dev/mapper/${MAPPER}")"
cryptsetup close "${MAPPER}"
note "read back via --header: ${got}"
if [[ "${got}" != "${PAYLOAD}" ]]; then
  echo "payload mismatch" >&2
  exit 1
fi

log "7. A backup taken from the live device is NOT a trust anchor"
cryptsetup luksHeaderBackup --header-backup-file "${HDR_LIVE}" "${DISK}"
if cmp -s "${HDR_TRUSTED}" "${HDR_LIVE}"; then
  note "this honest run: live backup == format-time backup (no tampering here)"
else
  note "live backup differs from format-time backup"
fi
note "in a CVM, repeating luksHeaderBackup at open time only copies whatever the host presents"
note "detached header helps only if you keep using the trusted copy (and MAC/measure it)"

log "8. Allow-list check on the format-time header JSON (defense, not a MAC)"
cryptsetup luksDump --dump-json-metadata --header "${HDR_TRUSTED}" "${DISK}" \
  >"${WORK}/trusted.json"
python3 "${ROOT}/validate_header_json.py" "${WORK}/trusted.json"

log "9. Untrusted-host path (header swap on a copy; no cipher downgrade)"
note "CVM host can write the first megabytes of the disk (cleartext header + keyslots)."
note "This step copies an honestly-formatted foreign LUKS2 header onto a copy of our disk."
note "That is the host's control-plane write. It is not a confidentiality-bypass recipe."
HOST_SRC="${WORK}/host-presented-header.img"
HOST_KEY="${WORK}/host-keyfile"
DISK_AFTER_HOST="${WORK}/disk-after-host-write.img"
printf 'host-chosen-passphrase\n' >"${HOST_KEY}"
chmod 600 "${HOST_KEY}"
dd if=/dev/zero of="${HOST_SRC}" bs=1M count=32 status=none
cryptsetup luksFormat --type luks2 --batch-mode \
  --pbkdf pbkdf2 --pbkdf-force-iterations 1000 \
  --key-file "${HOST_KEY}" "${HOST_SRC}"
cp --sparse=never "${DISK}" "${DISK_AFTER_HOST}"
# Default LUKS2 payload offset is 16 MiB; host replaces only the metadata region.
dd if="${HOST_SRC}" of="${DISK_AFTER_HOST}" bs=1M count=16 conv=notrunc status=none
note "wrote 16 MiB of host-controlled metadata onto ${DISK_AFTER_HOST}"

log "9a. Attached open follows the live (host-written) header — guest key must fail"
if cryptsetup open --key-file "${KEY}" "${DISK_AFTER_HOST}" "${MAPPER}" 2>"${WORK}/attached-after-host.err"; then
  cryptsetup close "${MAPPER}" || true
  echo "unexpected: attached open succeeded after host header swap" >&2
  exit 1
fi
note "attached open with the guest key failed (expected). cryptsetup saw the host header:"
sed 's/^/        /' "${WORK}/attached-after-host.err" | tail -n 5

log "9b. Detached open with the format-time header still reads our payload"
cryptsetup open --header "${HDR_TRUSTED}" --key-file "${KEY}" \
  "${DISK_AFTER_HOST}" "${MAPPER}"
got="$(head -n 1 "/dev/mapper/${MAPPER}")"
cryptsetup close "${MAPPER}"
note "detached open after host write: ${got}"
if [[ "${got}" != "${PAYLOAD}" ]]; then
  echo "payload mismatch after host header swap" >&2
  exit 1
fi
note "payload survived because it lives past the 16 MiB metadata region"
note "TOCTOU lesson: dump-then-open on the live device would have used the host header"

log "9c. Host-presented JSON profile is rejected (fixture, not applied to a volume)"
python3 - <<'PY' "${WORK}/trusted.json" "${WORK}/host-presented.json"
import json, sys
from pathlib import Path
src, dst = Path(sys.argv[1]), Path(sys.argv[2])
j = json.loads(src.read_text())
j["segments"]["0"]["encryption"] = "UNTRUSTED-HOST-CONTROLLED"
dst.write_text(json.dumps(j, indent=2) + "\n")
print("    wrote fixture", dst)
print("    this file is a policy input only; it is not written back onto the disk")
PY
if python3 "${ROOT}/validate_header_json.py" "${WORK}/host-presented.json"; then
  echo "unexpected: host-presented profile was accepted" >&2
  exit 1
else
  note "validator rejected the host-presented profile (expected)"
fi
note "a MAC or attestation measurement is still required; allow-lists are not authentication"

log "Done"
note "work directory: ${WORK}"
note "clean up with: $0 clean"
