# WhyBig

> Disk analyzers tell you what's big. WhyBig tells you what **got** big.

WhyBig is a disk-space **growth** tracker, not another disk usage analyzer.
It records snapshots of directory sizes over time so you can answer:

> "What ate my disk space recently?"

```console
$ whybig snapshot ~
...
$ whybig snapshot ~          # a few days later
...
$ whybig diff                # Milestone 2
Disk Growth   Sep 9 → Sep 12
Total          +17.4 GB
Grew
────────────────────────────
+11.8 GB    ~/.docker
 +3.2 GB    ~/Library/Caches
...
```

## Status: Milestone 1

Milestone 1 delivers the reliable core — a fast scanner, transactional
SQLite storage, and the `init` / `snapshot` / `status` commands. `diff` and
friends come in Milestone 2, only once scanning + storage are proven solid.

| Command | Purpose |
|---|---|
| `whybig init` | create + migrate the data directory / database (idempotent) |
| `whybig snapshot <path>` | record a directory-size snapshot |
| `whybig status` | show DB and latest snapshot info |

Not yet implemented (by design): `diff`, `inspect`, `history`, `top`, TUI,
GUI, daemon, auto-background scanning, deletion/cleanup, AI, networking, cloud
sync, telemetry, accounts, auto-modification of user files.

## Install / build

```console
# stable Rust (MSRV: current stable; edition 2021)
cargo install --path .
# or just run from the repo
cargo build --release && ./target/release/whybig --help
```

## Usage

```console
$ whybig init
WhyBig is ready.
  data directory: C:\Users\you\AppData\Roaming\whybig   # platform default
  database:       ...\whybig\whybig.sqlite3

$ whybig snapshot ~
Scanning ~ ...
2,491 files
312 directories
9.4 GB

Snapshot #1 saved in 2.4s

$ whybig status
Data directory: C:\Users\you\AppData\Roaming\whybig
Database:       ...\whybig\whybig.sqlite3 (624 KB)
Snapshots:      1
Earliest:       2026-09-08 14:20:03  (root: C:\Users\you)
Latest:         2026-09-08 14:20:03  (root: C:\Users\you)
  total size:    9.4 GB
  file count:    2,491
  directory count: 312
  scan duration: 2.4s
  skipped:       0  (size kind: apparent)
```

Options:

- `--data-dir <dir>` or `WHYBIG_DATA_DIR` — override where WhyBig stores data.

## How it works

```
Filesystem
    ↓
Scanner  ──────────────  pure data, no SQLite
    ↓
SnapshotService  ──────  orchestrates
    ↓
Storage  ──────────────  single-transaction SQLite write
```

- Scans everything, but only persists **directory-level aggregates**
  (`path`, `size`, `file_count`, `dir_count`, `skipped_count`) — never a row
  per file.
- Every snapshot save is **one transaction**: failure rolls back, so no
  half-snapshot can exist.
- Symlinks are never followed; Unix filesystem boundaries (`dev()`) are not
  crossed by default.
- Permission-denied / vanished entries are counted as `skipped` and reported —
  they never abort a scan.
- WhyBig's own data directory is excluded from any scan it overlaps.
- Sizes are **apparent** (logical) size in Milestone 1; the schema reserves
  `size_kind` for the future `allocated` dimension.

## Development

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Design notes, decisions and known limitations live in [`DESIGN.md`](DESIGN.md).
CI runs on Linux / macOS / Windows (see `.github/workflows/ci.yml`).

## License

MIT — see [`LICENSE`](LICENSE).
