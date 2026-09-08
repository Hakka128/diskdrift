# WhyBig Benchmarks

> Reproducible, honest wall-clock measurements of the core pipeline.
> These are **environment-specific, noisy numbers** (OS cache, AV, SSD-state
> dependent) — they are engineering datapoints, not product promises.

## How to reproduce

```console
# 10k (CI-style smoke)
cargo build --release
cargo run --release --example bench -- --files 10000 --mode mixed --seed 7

# 100k / 1M (local/manual; 1M takes minutes to *generate*)
cargo run --release --example bench -- --files 100000 --mode mixed --seed 7
cargo run --release --example bench -- --files 1000000 --mode mixed --seed 7
```

The runner generates a deterministic tree (same `--seed` ⇒ same tree), then
measures: generation, `scanner::scan` wall time + files/sec + dirs/sec, two
snapshot writes + total DB size (main + `-wal` + `-shm`), `diff`, `history(20)`.

Generator notes: tiny/mixed files hold 1–128 bytes (small disk footprint);
`large` mode uses `set_len` (sparse on Unix, real allocation on Windows).

## Baseline machine

| | |
|---|---|
| OS | Windows 11 Pro 10.0.22631 (64-bit) |
| Filesystem | NTFS (C:) |
| CPU | AMD Ryzen 7 9800X3D (8 cores / 8 threads) |
| Storage | Colorful CN700 2TB PRO (NVMe SSD) |
| Toolchain | rustc 1.98.1 stable-x86_64-pc-windows-msvc, `--release` |

## Results — tree shape `mixed`, seed 7

| metric | 10k | 100k | 1M |
|---|---|---|---|
| generated files / dirs | 10,000 / 600(→800) | 100,000 / 600(→800) | 1,000,000 / 600(→800) |
| generate | ~2.2 s | ~23.9 s | ~244 s (NTFS small-file creates) |
| **scan** | **55 ms** | **81 ms** | **1330 ms** |
| scan throughput | ~180k files/s | ~1.24 M files/s | ~752k files/s |
| scan total (files, dirs, bytes) | 10,000, 800, 0.64 MB | 100,000, 800, 6.4 MB | 1,000,000, 800, 64 MB |
| db write ×2 snapshots | ~98 ms | ~136 ms | ~484 ms |
| db size (main+wal+shm) | small | small | **~0.5 MB** (801 dir rows) |
| diff | ~1.6 ms | ~1.6 ms | ~1.5 ms |
| history (20) | <1 ms | <1 ms | <1 ms |

> "→800" — the generator counts an implied root dir; dirs reported by the scan.

100k is faster per-file than 1M (warm FS caches / metadata locality); both are
far inside the M1 "1M files ≈ 15 s" engineering target for tiny files.
**DB size for 1M files ≈ 0.5 MB** because only directory aggregates are stored
(no per-file rows) — the core architectural bet, now measured.

## Honesty notes

- CI only runs 10k; 100k/1M are local/manual datapoints from this machine.
- Scan favours tiny files (1–128 B): real-world trees with many small/medium
  files vary; a fresh `benches/` Criterion harness could add depth variants.
- Cold vs warm cache, AV, and drive fill matter; always re-measure on your own
  machine. Peak-memory RSS is not captured here (not stably obtainable cross-
  platform in-process); DB/snapshot sizes bound memory: entries are
  directory-level only.
- `generate` (small-file creates on NTFS) dominates small runs; it is
  **not** part of `snapshot` latency.

## Hotspot status (from the numbers)

- Scanner is already fast (~1.2 M tiny files/s on this machine); before adding
  rayon / `d_type` fast paths, profile a *real* tree (mixed sizes) with Criterion.
- Diff/history are single-query and sub-millisecond — no action needed.
