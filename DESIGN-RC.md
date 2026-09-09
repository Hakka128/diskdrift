# DiskDrift — v0.1.0 Release Candidate 设计记录 (WhyBig → DiskDrift)

> Feature-frozen hardening round. Nothing in this document introduces new
> product functionality; it records branding/public-release decisions so the
> RC can be reviewed and replayed.

## 1. Branding

- Product: **DiskDrift**; binary/crate/repo: **diskdrift**; tagline:
  *"Find what's quietly growing on your disk."*; core expression:
  *"Disk analyzers tell you what's big. DiskDrift tells you what got big."*
- Forbidden spellings: Diskdrift / Disk Drift / diskDrift / DISK_DRIFT (except
  the ordinary phrase "disk drift" in natural language).

## 2. Legacy (pre-release “WhyBig”) data strategy — no automatic migration

WhyBig was never publicly released; the DB was only ever created by this
repository's local dev builds. Auto-migrating on that basis would add
destructive complexity for zero real-world users, so we chose **manual,
explicit migration** — the smallest, safest, most testable option.

Rules:
- DiskDrift **never** deletes, moves or auto-merges legacy data.
- `diskdrift init` **detects** a legacy WhyBig data directory/database and
  prints a one-time note on stderr pointing at manual migration steps; stdout
  stays clean (JSON-safe).
- If both the new and the legacy data directory exist, DiskDrift uses the new
  one and does **not** guess or merge.
- Legacy env `WHYBIG_DATA_DIR` is honored as a **deprecated fallback** behind
  `DISKDRIFT_DATA_DIR` (priority: `--data-dir` → `DISKDRIFT_DATA_DIR` →
  `WHYBIG_DATA_DIR` → platform default), with a stderr-only deprecation note.

### Manual migration (documented for anyone with a dev-era WhyBig DB)

```console
# 1) stop using the old data dir
# 2) copy + rename the old database into the new layout
cp "$APPDATA/whybig/whybig.sqlite3" "$APPDATA/diskdrift/diskdrift.db"   # Windows
# macOS: ~/Library/Application Support/whybig → .../diskdrift
# Linux: $XDG_DATA_HOME/whybig → .../diskdrift (or ~/.local/share/...)
# 3) open it with the new binary — schema migration runs in place
diskdrift init
diskdrift status
```

The copied DB keeps `snapshots`/`entries` ids and schema version; opening with
DiskDrift upgrades it (v1→v2) inside normal migrations. The legacy file is
untouched.

## 3. Environment variable

| priority | source |
|---|---|
| 1 | `--data-dir <dir>` |
| 2 | `DISKDRIFT_DATA_DIR` |
| 3 | `WHYBIG_DATA_DIR` (deprecated; stderr note) |
| 4 | platform default (`%APPDATA%/diskdrift`, `~/Library/Application Support/diskdrift`, `$XDG_DATA_HOME/diskdrift`) |

`cli` no longer attaches `env=` to the flag; ordering lives entirely in
`config::data_dir` (one rule, no surprises).

## 4. Database

- Default file: **`diskdrift.db`**.
- **No schema change for branding**: `snapshots`/`entries`/`meta` unchanged,
  latest `user_version` stays 2, no new migration was added.
- `meta` has no `application_name` field, so there is nothing to migrate.

## 5. JSON

- `schema_version` stays **1** (no breaking schema change).
- `command` values unchanged: snapshot/status/diff/inspect/history/top/prune.
- No `application`/`tool` fields added. `status`'s `data_dir`/`database_path`
  simply reflect the new paths.
- Legacy deprecation notices go to **stderr** only; JSON stdout stays pure
  (tested).

## 6. Cargo / artifacts

- `name=diskdrift`, version `0.1.0`, description *"A local-first disk growth
  debugger that shows what got big over time."*, keywords
  disk/filesystem/storage/cli/monitoring, categories
  command-line-utilities+filesystem.
- Release pipeline (`.github/workflows/release.yml`) triggers on tags
  `v0.1.0-rc.*` and `v0.1.0`; builds **Windows x86_64** only, packages
  `diskdrift-v0.1.0-x86_64-pc-windows-msvc.zip` (+ `.sha256`), uploads to the
  GitHub release. Ubuntu remains as CI portability coverage, not a release
  target.
- First validate the pipeline with `v0.1.0-rc.1` before approving `v0.1.0`.

## 7. What deliberately keeps the old name (intentional)

- `DESIGN.md`, `DESIGN-M2.md`, `DESIGN-M3.md`, `DESIGN-M4.md` — historical
  design documents recorded under the original code name.
- Git history/logs (author names, commit messages, dev email).
- `config::legacy_data_dir/legacy_db_path` — the strings "whybig"/"whybig.sqlite3"
  exist only to *detect* old data.
- The "WHYBIG_DATA_DIR" lookup — the deprecated fallback itself.

## 8. Release blockers, if any

Tracked in the RC report; not anticipated beyond: actual crates.io publish
(README will not claim `cargo install diskdrift` until publish succeeds), and
the first real `v0.1.0-rc.1` pipeline run.
