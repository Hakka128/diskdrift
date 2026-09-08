# WhyBig

> Disk analyzers tell you what's big.
> WhyBig tells you what **got** big.

WhyBig is a **disk growth tracker**, not another disk usage analyzer. It records
directory-size snapshots over time and shows you *what ate your disk*, so you
can answer: **"my disk grew 17 GB this week — where did it go?"**

No file deletion. No background service. No cloud, telemetry or accounts. It
observes, records, compares and explains.

---

## Demo

Real output from `examples/demo.rs` (deterministic fixture, real binary paths):

```console
$ whybig snapshot ~/demo-root
4 files
7 directories
3.7 KB

Snapshot #1 saved in 0.0s

# … a few days later …

$ whybig snapshot ~/demo-root
5 files
5 directories
7.8 KB

Snapshot #2 saved in 0.0s

$ whybig diff
Disk Growth
2026-09-09 06:17:06 → 2026-09-09 06:17:06
root: /home/you/demo-root

Total
  before:  3.7 KB
  after:   7.8 KB
  delta:   +4.1 KB

Grew
────────────────────────
       +2.9 KB  docker
       +1.5 KB  downloads

Shrank
────────────────────────
        -300 B  projects

$ whybig inspect ~/demo-root/docker
/home/you/demo-root/docker
root: /home/you/demo-root

2026-09-09 06:17:06  1.5 KB  (snapshot 1)
2026-09-09 06:17:06  4.4 KB  (snapshot 2)

Total growth
+2.9 KB

Contributors
────────────────────────
       +2.9 KB  containers/
           0 B  other

$ whybig top
Top Disk Growth
Sep 09 → Sep 09
       +2.9 KB  docker
       +1.5 KB  downloads
```

## Problem

`du` shows you what's *big* right now. It has no memory. WhyBig keeps a
**timeline of directory-level sizes** (a few KB per snapshot in SQLite) and
answers the questions users actually ask:

- `whybig diff` — "between two moments, which top-level directories changed?"
- `whybig inspect <dir>` — "inside *that* block, which child grew?"
- `whybig history` — "how has this directory trended over time?"
- `whybig top` — "who grew the most recently?"

## Quick Start

```console
$ whybig init
$ whybig snapshot ~        # record a baseline
# … later …
$ whybig snapshot ~        # record another
$ whybig diff --since 7d   # what changed in the last ~7 days
```

Prune old snapshots (WhyBig's own history only — never your files):

```console
$ whybig prune             # dry-run preview, deletes nothing
$ whybig prune --apply     # apply the retention policy
$ whybig compact           # explicitly shrink the SQLite file (VACUUM)
```

Machine-readable output everywhere: `--json` on `diff`, `inspect`, `history`,
`top`, `status`, `snapshot` and `prune` (stable schema `schema_version: 1`).

## Commands

| Command | Answers |
|---|---|
| `snapshot <path>` | "What's there now?" — record directory sizes |
| `status` | data dir, DB size, latest snapshot summary |
| `diff [--since 7d \| --from ID --to ID]` | "Between two times, what changed at the top level?" |
| `inspect <path>` | "Inside this block, which child changed?" (plus `other` residual) |
| `history [<path>] [--limit N]` | "How has this grown over time?" |
| `top [<path>] [--shrink]` | "Who grew (or freed) the most recently?" |
| `prune [--apply]` | Preview/apply snapshot retention (dry-run by default) |
| `compact` | Explicitly VACUUM the database file |

Common flags: `--data-dir <dir>` / `WHYBIG_DATA_DIR`, `--json` (stable output),
`--limit N`, `--since 30m\|24h\|7d\|4w`.

## How it works

```
Filesystem → Scanner → SnapshotService → SQLite (directory-level only)
                                ↓
                diff / inspect / history / top → Human or JSON
```

- Scans everything, but **stores only directory aggregates**
  (`path, size, file_count, dir_count, skipped`) — never a row per file.
  A 1M-file snapshot is ~0.5 MB in SQLite.
- Each snapshot write is **one transaction** (no half-snapshots).
- Symlinks are never followed; Unix filesystems (`dev()`) are not crossed.
- Permission-denied / vanished entries are counted as `skipped`, never fatal.
- WhyBig's own data directory is excluded from any scan it overlaps.
- Sizes are apparent (logical) bytes; `size_kind` reserves `allocated`.
- Retention (`prune`) buckets by **UTC** days/weeks, always keeps each root's
  latest snapshot, and deletes inside a single transaction (FK cascade).
- `--since` selection uses UTC timestamps and explicitly tells you when it had
  to fall back to the earliest snapshot.

## Privacy

- **Local only.** No network, no cloud, no telemetry, no accounts.
- WhyBig **reads filenames and sizes**, never file contents.
- It stores **directory-level size metadata** in your own SQLite database.
- `prune` only removes WhyBig's own snapshots — it **never deletes or modifies
  your files**. No background service, no file watcher, no auto-deletion.

## Performance

Local benchmark (Windows 11, NTFS, Ryzen 7 9800X3D, release build,
`examples/bench`): a **1,000,000-file** auto-generated tree scans in **~1.3 s**
(~750k files/s, tiny files), two snapshot writes ~0.5 s, and the database ends
up **~0.5 MB** because only directory aggregates are stored. Details in
[`BENCHMARKS.md`](BENCHMARKS.md) (environment-specific numbers, worst-case of
Windows caching; not a guarantee).

## Limitations

- Sizes are **directory-level**: WhyBig tells you which directory grew, not
  which individual file (by design — see Roadmap for file-level attribution).
- Windows does not have a cheap device-id, so filesystem-mount boundaries are
  not detected on Windows (junction-heavy setups may behave differently).
- Non-UTF-8 filenames are stored as deterministic lossy UTF-8 keys.
- `--since` accepts whole units (`m`/`h`/`d`/`w`) only, no calendar math.
- Peak memory is not measured cross-platform; memory is bounded by directory
  count (never by file count).

## Installation

```console
cargo install --path .        # from a release checkout
# or
cargo build --release && ./target/release/whybig --help
```

Runs on Linux, macOS and Windows (Rust stable, edition 2021; MSVC/GNU on
Windows). CI builds and tests all three platforms.

## Roadmap

- **v0.1.0** (this): snapshot · status · diff · inspect · history · top · prune
  · JSON API v1 · benchmark framework.
- Next: file-level attribution for small directories (optional, opt-in),
  `diff --since` extra units, automated retention scheduling (explicit,
  user-approved), `top --since 30d` polish, and Windows mount-boundary support.

---

WhyBig observes and explains disk growth. It does not clean your computer.
