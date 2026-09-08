# Contributing to DiskDrift

Thanks for helping make DiskDrift trustworthy and useful. This is a small,
local-first project; keep changes focused and review-friendly.

## Ground rules

- **No feature creep outside the roadmap.** DiskDrift is a disk growth
  *debugger*: it records, compares, and explains. It does not clean users'
  computers, read file contents, or phone home.
- Feature-frozen during release hardening: branding/docs/release changes take
  priority over new product functionality.

## Getting started

```console
git clone https://github.com/diskdrift/diskdrift
cd diskdrift
cargo build
cargo test
```

Use `cargo run -- release`-style demos via `examples/demo.rs` and benchmarks
via `examples/bench` (`BENCHMARKS.md`).

## Before opening a PR

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test`
4. For behavior changes: **add a failing test first**, then implement.
5. Update `CHANGELOG.md` under `[Unreleased]` where user-visible, and adjust
   docs/README if commands or defaults change.

## Testing guidance

- Filesystem tests use `tempfile`; big-file tests stay modest (Windows
  `set_len` allocates real space — see notes in tests).
- `--json` changes must keep stdout a single JSON document (no progress bars,
  warnings only on stderr); add golden assertions in `tests/json_golden.rs`.
- Keep integration tests independent of the real data directory (use
  `--data-dir` / temp dirs).

## Branches / releases

- `main` is the default branch; PRs target `main`.
- Release candidates are tagged `v0.1.0-rc.N`, then `v0.1.0`; the release
  workflow builds and checksums artifacts automatically.

## Code of conduct

Be constructive and respectful. Disagreements are welcome; ad hominem is not.
