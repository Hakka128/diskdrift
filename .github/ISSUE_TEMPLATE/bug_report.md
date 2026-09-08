---
name: Bug report
about: Report a problem with DiskDrift
title: "[BUG] "
labels: bug
assignees: ""
---

**Describe the bug**
A clear and concise description of what happened.

**Command(s) run**
```console
$ diskdrift ...
```

**Expected vs actual**
What did you expect? What happened instead? Paste terminal output (redact any
sensitive paths).

**Environment**
- OS / version (e.g. Windows 11 23H2, Ubuntu 24.04, macOS 15)
- Filesystem (e.g. NTFS, ext4, APFS) and storage type (SSD/HDD/NVMe)
- DiskDrift version (`diskdrift --version`)

**Data safety note**
If this involves data loss, unexpected deletion, or a prune/compact behavior,
please follow [SECURITY.md](SECURITY.md) for private reporting instead.

**Additional context**
Any details about the directory layout, symlinks, mounts, non-ASCII paths, or
logs that help reproduce.
