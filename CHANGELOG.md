# Changelog

All notable changes to DiskDrift are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

No unreleased changes yet.

## [0.1.0] — 2026-09-?? (Release Candidate)

Initial public release.

### Added

- `diskdrift snapshot <path>` — record directory-level disk usage over time
  (apparent sizes, directory aggregates only).
- `diskdrift status` — data directory, database size, latest snapshot summary.
- `diskdrift diff` — compare two snapshots and attribute growth to top-level
  directories (`--from/--to`, `--since 30m|24h|7d|4w`, `--limit`, `--all`).
- `diskdrift inspect <path>` — drill into a directory's direct contributors,
  including the `other` residual.
- `diskdrift history [path]` — a directory's size trend across snapshots.
- `diskdrift top [path]` — rank the biggest growth (or, with `--shrink`, the
  most space freed).
- `diskdrift prune` — dry-run by default; `--apply` enforces a UTC-bucketed
  retention policy (recent daily weekly tiers) that only ever removes
  DiskDrift's own snapshots.
- `diskdrift compact` — explicit `VACUUM` to shrink the SQLite file.
- Stable JSON output (`--json`) for snapshot/status/diff/inspect/history/
  top/prune with `schema_version: 1` and RFC3339 UTC timestamps.
- Officially supported platforms: Windows and Linux.

### Safety

- Local-first: no network, no telemetry, no cloud, no accounts.
- Excludes its own data directory from scans; never deletes user files.

[0.1.0]: https://github.com/diskdrift/diskdrift/releases/tag/v0.1.0
