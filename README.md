# confidential-disk-format

This repo summarizes the disk format proposed for the CoCo community. The basic target is

1. (Optional) Confidentiality and integrity protection. Optional as a pair: a volume may have neither, integrity alone, or both — but **never confidentiality without integrity**, because unauthenticated AES-XTS lets a host that can write the disk alter plaintext undetectably.
2. Versatility. Means not only CoCo could use it, CVM scenarios w/o CoCo still works.
3. Self-containerized. Metadata is transparent to users. The image describes how to interpret itself; it does **not** carry the key or the key locator, both of which come from the deployment.
4. Out-of-box.
5. Authentic open contract. The metadata that decides how the volume is opened must be authenticated with a key the host does not hold. Authenticated, not encrypted — nothing in that metadata is secret, so CDF v1 MACs it in cleartext ([design §7.1](design/README.md#71-why-a-mac-and-not-aead)).

Explicitly **out of scope**: freshness of the whole device. A host that restores an older, internally consistent copy of the image defeats every on-disk check, because it rolls back the evidence together with the data. Detecting that needs mutable state outside the disk, so it belongs to the deployment rather than to the format. See [investigate/README.md §1.2](investigate/README.md).

The survey of existing formats is in [investigate/README.md](investigate/README.md) ([中文](investigate/README.zh.md)). No existing standard meets all five goals. The format specified here is **CDF v1**: [design/README.md](design/README.md) ([中文](design/README.zh.md)).
