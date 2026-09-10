#!/usr/bin/env python3
"""Fail closed unless LUKS2 JSON uses an agreed data-plane cipher.

This is a policy check on already-parsed metadata. It is not a MAC, and it
does not open or modify a volume.
"""
from __future__ import annotations

import json
import sys

ALLOWED_ENCRYPTION = frozenset({"aes-xts-plain64"})


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} header.json", file=sys.stderr)
        return 2
    data = json.loads(open(argv[1], encoding="utf-8").read())
    enc = data["segments"]["0"].get("encryption")
    ok = enc in ALLOWED_ENCRYPTION
    print(f"    encryption {enc!r} in allow-list {sorted(ALLOWED_ENCRYPTION)}: {ok}")
    if not ok:
        print("    reject: host-controlled metadata names a cipher this guest will not activate")
        return 1
    print("    allow-listing is a policy check; it is not cryptographic authentication")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
