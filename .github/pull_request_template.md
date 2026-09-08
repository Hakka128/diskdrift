<!-- Thank you for contributing! Format must pass, clippy must be clean,
     and tests must be green (`cargo fmt --check`, `cargo clippy --all-targets
     --all-features -- -D warnings`, `cargo test`). -->

## Summary
What this PR does and why.

## Change type
- [ ] Bug fix
- [ ] Docs / branding / release
- [ ] Enhancement (feature-frozen candidate: please note why it must ship now)
- [ ] Refactor / chore

## Testing
- [ ] Added/updated tests (a failing test first for behavior changes)
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo test` passes

## Data-safety note
DiskDrift must never delete or modify user files outside its own snapshots.
Call out anything touching `prune`/`compact`/paths.

## Changelog
- [ ] Added a line to `CHANGELOG.md` under `[Unreleased]` (if user-visible)
