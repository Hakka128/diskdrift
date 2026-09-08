<div align="center">

# DiskDrift

**Find what's quietly growing on your disk.**

Disk analyzers tell you what's big.
**DiskDrift tells you what got big.**

*A fast, local-first disk growth debugger that shows what got big over time.*

</div>

---

## Demo

Real output from `examples/demo.rs` (deterministic fixture, real binary):

```console
$ diskdrift snapshot ~/demo-root
4 files
7 directories
3.7 KB

Snapshot #1 saved in 0.0s

# … a few days later …

$ diskdrift snapshot ~/demo-root
5 files
5 directories
7.8 KB

Snapshot #2 saved in 0.0s

$ diskdrift diff
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

$ diskdrift inspect ~/demo-root/docker
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

$ diskdrift top
Top Disk Growth
Sep 09 → Sep 09
       +2.9 KB  docker
       +1.5 KB  downloads
```

## Why DiskDrift?

`du` shows what's *big right now*. It has no memory, so it cannot tell you
that 17 GB quietly appeared in `~/.docker` over two weeks. DiskDrift records
**directory-level sizes over time** (a few KB per snapshot in a local SQLite
file) and answers the questions people actually ask:

- `diskdrift diff` — "between two moments, which top-level directories changed?"
- `diskdrift inspect <dir>` — "inside *that* block, which child grew?"
- `diskdrift history` — "how has this directory trended over time?"
- `diskdrift top` — "who grew the most recently?"

## Quick Start

```console
$ diskdrift init
$ diskdrift snapshot ~        # record a baseline
# … later …
$ diskdrift snapshot ~        # record another
$ diskdrift diff --since 7d   # what changed in the last ~7 days
```

Keep the history tidy (removes DiskDrift's own snapshots only — never your
files):

```console
$ diskdrift prune             # dry-run preview, deletes nothing
$ diskdrift prune --apply     # apply the retention policy
$ diskdrift compact           # explicitly shrink the SQLite file (VACUUM)
```

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

Common flags: `--data-dir <dir>`, `--json` (stable output), `--limit N`,
`--since 30m|24h|7d|4w`. Environment: `DISKDRIFT_DATA_DIR`.

## How it works

```
Filesystem → Scanner → SnapshotService → SQLite (directory-level only)
                                ↓
                diff / inspect / history / top → Human or JSON
```

- Scans everything, but **stores only directory aggregates**
  (`path, size, file_count, dir_count, skipped`) — never a row per file.
- Each snapshot write is **one transaction** (no half-snapshots).
- Symlinks are never followed; Unix filesystems (`dev()`) are not crossed.
- Permission-denied / vanished entries become `skipped`, never fatal.
- DiskDrift's own data directory is excluded from any scan it overlaps.
- Sizes are apparent (logical) bytes; `size_kind` reserves `allocated`.
- Retention (`prune`) buckets by **UTC** days/weeks, always keeps each root's
  latest snapshot, deletes in one transaction (FK cascade).
- `--since` uses UTC timestamps and honestly tells you when it fell back to
  the earliest snapshot.

## JSON / scripting

Every command supports `--json` with a stable, versioned schema
(`"schema_version": 1`); stdout contains exactly one JSON document
(no progress bars, no warnings on stdout). Sizes are integer bytes;
timestamps are RFC3339 UTC. Useful for `jq` pipelines and dashboards.

```console
$ diskdrift top --json | jq '.[]'           # every mode is machine-readable
$ diskdrift snapshot ~ --json               # record + get created_at
```

## Privacy

- **Local-first.** No network, no cloud, no telemetry, no accounts.
- Reads **filenames and sizes only** — never file contents.
- Stores **directory-level size metadata** in your own SQLite database
  (`diskdrift.db` in the data directory).
- `prune` only deletes **DiskDrift's own historical snapshots**; it never
  deletes or modifies your files.
- No background service, no file watcher, no auto-deletion, no auto-upload.

## Performance

Measured on one reference machine (Windows 11, NTFS, Ryzen 7 9800X3D, NVMe,
`--release`): a 1,000,000-file synthetic tree scans in **~1.3 s**, two
snapshot writes ~0.5 s, and the resulting database is **~0.5 MB** because only
directory aggregates are stored.

Results vary significantly by filesystem, hardware, cache state, antivirus,
directory layout, and OS — see [`BENCHMARKS.md`](BENCHMARKS.md) for the full
methodology and raw numbers.

## Installation

From a release binary (recommended): download the archive for your platform
from the [Releases](https://github.com/diskdrift/diskdrift/releases) page and
verify against `SHA256SUMS`.

From source:

```console
cargo build --release
./target/release/diskdrift --help
```

*(`cargo install diskdrift` will be listed here once the crate is published.)*

Runs on Linux, macOS and Windows (Rust stable, edition 2021). CI builds and
tests all three platforms.

## Limitations

- Sizes are **directory-level**: DiskDrift tells you which directory grew, not
  which individual file (by design — see Roadmap).
- Windows has no cheap device-id in `std`, so **filesystem-mount/junction
  boundaries are not detected on Windows** (a junction into another volume is
  followed like a normal directory).
- Non-UTF-8 filenames are stored as **deterministic lossy UTF-8** keys.
- Windows Unicode case-folding has edge cases: a differently-cased path that
  no longer exists may read as 0 in history.
- **No file-level historical attribution**; only directory aggregates persist.
- `--since` accepts whole units (`m`/`h`/`d`/`w`) only — no calendar math.
- Filesystem snapshots are **best-effort, not atomic**: sizes are a point-in-
  time sample; racing scans may miss a concurrent change (documented).

## Roadmap

- **v0.1.0 (this release)**: snapshot · status · diff · inspect · history ·
  top · prune · compact · JSON API v1 · benchmark framework.
- Next: opt-in file-level attribution for small directories; more `--since`
  units; scheduled retention (explicit, user-approved); Windows
  mount-boundary support.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Report security issues per
[`SECURITY.md`](SECURITY.md).

## License

MIT — see [`LICENSE`](LICENSE).

---

DiskDrift records, compares, and explains disk growth. It does not clean your
computer, read your file contents, or send data anywhere.
