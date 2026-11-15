---
title: Security Model & Threat Assessment
description: Trust boundaries, process privileges, and archive extraction security in Crux.
---

## Process Privileges & Sandbox Reality

> **Important Disclosure**: In Crux v0.1.0, external build tools (`cmake`, `make`, `ninja`, compilers) execute with the ambient privileges of the user running `crux`. No operating-system containerization or namespace sandbox (seccomp/bubblewrap) is active in v0.1.0.

### Mitigations in Place
- **Zero Shell Interpolation**: All external commands (`git`, `cmake`, compilers) are executed with discrete argument vectors (`std::process::Command::arg`). Crux never invokes shell wrappers (`sh -c` or `cmd.exe /C`).
- **Working Directory Confinement**: Subprocesses are strictly confined to dedicated working directories.
- **Path Escaping**: Generated CMake scripts escape all user-controlled paths and reject embedded newlines or null bytes.

---

## Archive Extraction Defenses

Crux enforces defense-in-depth protections during `.zip`, `.tar`, and `.tar.gz` extraction:
1. **Zip Slip Protection**: Destination paths are canonicalized and verified to reside strictly within the target root directory.
2. **Symlink Escape Jail**: Symlink targets pointing outside extraction or staging roots are rejected.
3. **Decompression Bomb Ceilings**: Maximum 500 MB uncompressed size, maximum 50,000 files, and maximum 100:1 compression expansion ratio.

---

## Cryptographic Digest Verification

- All remote package downloads require HTTPS.
- Downloaded bytes are streamed through SHA-256 hashers in real time.
- Checksums are verified before cache persistence or extraction. Corrupted or mismatched archives are immediately purged.
