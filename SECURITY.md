# Security Policy

DiskDrift is a filesystem tool: it scans directories and stores size metadata
locally. We take the safety of your data seriously.

## Reporting a vulnerability

Please **do not** open a public issue for security problems.

- If you have a GitHub account, report privately through
  **GitHub Security Advisories** on this repository
  (`https://github.com/diskdrift/diskdrift/security/advisories`).
- If Security Advisories are not available, open an issue titled
  `[SECURITY]` with a clear description and contact GitHub support to enable
  private reporting — do not disclose details publicly.

Include:

- what you did, what you expected, and what happened;
- the command lines and environment (OS, version, data-dir layout);
- whether any data was lost, modified, or transmitted.

## What we care about

- **Path traversal** — paths that could escape a scan/root and touch files the
  user did not intend.
- **Unintended deletion** — `prune --apply` must only ever remove DiskDrift's
  own snapshots, never user files; any path that could delete/move user data
  is critical.
- **Database corruption** — migration or write paths that could corrupt or
  silently lose snapshot data.
- **Privilege-related issues** — running from elevated contexts, symlink /
  mount-boundary handling that behaves unsafely.
- **Unsafe filesystem behavior** — TOCTOU races, symlink loops, crossing
  filesystems unexpectedly, or following attacker-controlled links.

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x (current release) | ✅ |
| older / pre-release | ❌ (security fixes land on the latest release only) |

## Response

We aim to acknowledge reports within 7 days and release a fix in a patch
release as soon as it is safe to do so.

## Safe by design

DiskDrift is local-first: no networking, no telemetry, no cloud, no automatic
data deletion. It reads filenames and sizes, never file contents.
